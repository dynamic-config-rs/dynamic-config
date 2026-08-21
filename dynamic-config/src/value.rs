//! An owned mirror of the resolved configuration tree.
//!
//! A boundary that is not `serde` — a language binding, an exporter, a
//! templating engine — needs the resolved values as *data*, not as a type
//! to deserialize into. The underlying loader has such a tree, but its
//! types are figment's, and this crate's public surface keeps figment
//! behind [one deliberate door](crate::Source::provider). So the export is
//! a small owned mirror: seven shapes, no lifetimes, no third-party types
//! in the signature — and built by walking the resolved tree directly,
//! never by a JSON round trip.
//!
//! # It is also the schemaless configuration
//!
//! [`Value`] implements `Deserialize`, which is the only bound the engine
//! puts on a configuration type — so `Dynamic<Value>`, `Builder::values`
//! and `load::<Value>` are a configuration with no struct behind it,
//! reading by path instead of by field. Nothing else in the engine changes:
//! the layering, the watcher, the cache and the reload hooks never knew
//! what `T` was. See [the book's schemaless
//! chapter](https://dynamic-config-rs.github.io/schemaless.html) for
//! what a struct still buys that this does not.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;

/// One resolved configuration value, owned.
///
/// What [`Snapshot::to_value`](crate::Snapshot::to_value) returns. This is
/// configuration *handover*, not a diagnostic: real values, secrets
/// included, exactly like deserializing into a struct — the paths-only
/// rule governs what this crate prints, not what it hands the program.
///
/// Which is why `Debug` is hand-written and shape-only: the same data
/// sits inside [`Snapshot`](crate::Snapshot), whose `Debug` prints keys
/// and never values, and `{:?}` in a log line is exactly how resolved
/// secrets leak. Read values through the enum; print them on purpose or
/// not at all.
///
/// **There is deliberately no `Display`.** A schemaless configuration has
/// no `#[config(secret)]` to derive a redaction list from, so a type that
/// rendered itself into `{}` would put a password wherever a program
/// formats a value it did not inspect — and it would do it in the one shape
/// (`format!`, `write!`, a template) where nothing looks like a decision.
/// The ways out are all explicit: the accessors, [`get_as`](Self::get_as),
/// [`render`](Self::render) for a document, and `Serialize` for a
/// serializer the caller chose.
#[derive(Clone, PartialEq)]
pub enum Value {
    /// An explicit null (or unit) in a source.
    Null,
    /// A boolean.
    Bool(bool),
    /// Any integer a source can express.
    ///
    /// `i128`, so every `i64` and `u64` fits without a sign decision at
    /// this boundary. The one unrepresentable case — a `u128` above
    /// `i128::MAX` — arrives as [`Value::Float`], lossily; a configuration
    /// value up there is measuring something no unit this crate knows
    /// about.
    Integer(i128),
    /// A floating-point number.
    Float(f64),
    /// A string; a single character in a source arrives as one too.
    String(String),
    /// A sequence.
    Array(Vec<Value>),
    /// A table, keyed by field name.
    Table(BTreeMap<String, Value>),
}

impl Value {
    /// What kind of thing this is, in the words a diagnostic uses.
    ///
    /// The kind and never the value: a message is the one place a
    /// configuration value has no business appearing, and "a string" is the
    /// whole of what a reader needs to know about the thing that was in the
    /// wrong place.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Value::Null => "nothing",
            Value::Bool(_) => "a boolean",
            Value::Integer(_) | Value::Float(_) => "a number",
            Value::String(_) => "a string",
            Value::Array(_) => "a list",
            Value::Table(_) => "a table",
        }
    }

    /// The value at a dotted `path` below this one, if every step exists.
    ///
    /// Steps are table keys; anything else — an array, a leaf — ends the
    /// walk with `None`. The empty path is this value itself.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Value> {
        if path.is_empty() {
            return Some(self);
        }

        path.split('.').try_fold(self, |value, step| match value {
            Value::Table(table) => table.get(step),
            _ => None,
        })
    }

    /// The value at a dotted `path`, deserialized into `T`.
    ///
    /// The convenient door onto a schemaless configuration, and the more
    /// expensive one. [`get`](Self::get) walks the tree and hands back a
    /// borrow; this walks it, rebuilds the value figment's deserializer
    /// wants and runs serde over it — **on every call**. Measured against
    /// the borrowed read on the same machine in the same run
    /// (`benches/read_path.rs`), that is around a third again as long for a
    /// scalar, and it allocates whatever the value it hands back owns: a
    /// number, nothing; a `String`, one.
    ///
    /// The bigger reason to prefer the accessors is not the nanoseconds but
    /// the `Result`: `get_as` is a conversion that can fail at every read,
    /// which is a diagnostic-grade shape. Use it where a value is read once
    /// at startup or per reload — a serde type the accessors cannot express,
    /// a `Vec<String>`, a struct for one sub-tree — and
    /// [`get`](Self::get) plus [`as_i64`](Self::as_i64) and friends on a
    /// request path. It is the same trade
    /// [`Snapshot::get`](crate::Snapshot::get) makes, written down where the
    /// schemaless reader will meet it.
    ///
    /// ```
    /// # #[cfg(feature = "json")] {
    /// use dynamic_config::{Format, Value};
    ///
    /// let document = Value::parse(r#"{"pool": {"max_size": 32}}"#, Format::Json).unwrap();
    ///
    /// assert_eq!(document.get_as::<u16>("pool.max_size").unwrap(), 32);
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Missing`](crate::ErrorKind::Missing) when nothing
    /// supplies `path` — including a path that walks *through* a scalar —
    /// and [`ErrorKind::Type`](crate::ErrorKind::Type) when what is there
    /// cannot become `T`. The message names the path and the kind of thing
    /// that was there, never the value.
    pub fn get_as<T: DeserializeOwned>(&self, path: &str) -> Result<T, crate::Error> {
        let value = self.get(path).ok_or_else(|| {
            crate::Error::new(crate::ErrorKind::Missing, "no value at this path").prepend_key(path)
        })?;

        // Through the crate's one reader, so a password typed into a numeric
        // field does not come back inside ``found string "hunter2"``: the
        // message names the kind that was there and never what it held.
        T::deserialize(crate::de::Reader(value))
            .map_err(|error| crate::de::Error::into_error(error).prepend_key(path))
    }

    /// The boolean here, or `None` if this is anything else.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(boolean) => Some(*boolean),
            _ => None,
        }
    }

    /// The integer here, at the width this crate stores it in.
    ///
    /// `None` for a float, even one that is a whole number: which of the two
    /// a source wrote is part of the configuration here, and
    /// [`Value::Integer`]'s `i128` is what makes that distinction free of a
    /// sign decision.
    #[must_use]
    pub fn as_integer(&self) -> Option<i128> {
        match self {
            Value::Integer(number) => Some(*number),
            _ => None,
        }
    }

    /// The integer here as an `i64`, or `None` if it is not one or does not
    /// fit.
    ///
    /// Narrowing rather than saturating: a port number that does not fit is
    /// a configuration mistake, and a clamped one is that mistake made
    /// silent.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        self.as_integer()
            .and_then(|number| i64::try_from(number).ok())
    }

    /// The integer here as a `u64`, or `None` if it is not one, is negative,
    /// or does not fit.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        self.as_integer()
            .and_then(|number| u64::try_from(number).ok())
    }

    /// The float here, or `None` if this is anything else — an integer
    /// included, for the reason [`as_integer`](Self::as_integer) gives.
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(number) => Some(*number),
            _ => None,
        }
    }

    /// The string here, borrowed, or `None` if this is anything else.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    /// The sequence here, borrowed, or `None` if this is anything else.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(values) => Some(values),
            _ => None,
        }
    }

    /// The table here, borrowed, or `None` if this is anything else.
    #[must_use]
    pub fn as_table(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Table(table) => Some(table),
            _ => None,
        }
    }

    /// The dotted path of every leaf, in order.
    ///
    /// What a schemaless configuration has instead of a field list: the keys
    /// that are actually there, learned at runtime. The same walk
    /// [`Snapshot::leaf_paths`](crate::Snapshot::leaf_paths) performs, on
    /// the tree a reader already holds — an array is a leaf, because its
    /// elements are values rather than configuration keys, and so is an
    /// empty table, which would otherwise vanish from the listing.
    ///
    /// Paths carry no values, so this is the one listing of a resolved
    /// configuration that is safe to log.
    ///
    /// A tree that is not a table has no paths: a document has named keys at
    /// its root.
    #[must_use]
    pub fn leaf_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();

        if let Value::Table(table) = self {
            let mut path = Vec::new();

            for (key, value) in table {
                path.push(key.clone());
                leaves(value, &mut path, &mut paths);
                path.pop();
            }
        }

        paths
    }

    /// Parses one `format` document into a tree.
    ///
    /// The way in to the parsing this crate already owns, for code that has
    /// documents to combine *before* the loader sees them: a store crate that
    /// reads several keys under a prefix, a tool that folds a fragment
    /// directory into one file. Without it the only way to merge two documents
    /// outside this crate is to depend on `serde_json`, `toml` and `serde_yaml`
    /// directly and reimplement what the `json` / `toml` / `yaml` features are
    /// already compiling.
    ///
    /// No section mapping is applied: the result is the document as written,
    /// top-level keys and all. Sections are what the *loader* does with a
    /// document, and a merge happens below that line.
    ///
    /// ```
    /// # #[cfg(feature = "json")] {
    /// use dynamic_config::{Format, Value};
    ///
    /// let mut document = Value::parse(r#"{"db": {"host": "a"}}"#, Format::Json).unwrap();
    /// document.merge(Value::parse(r#"{"db": {"port": 5432}}"#, Format::Json).unwrap());
    ///
    /// assert_eq!(document.get("db.host"), Some(&Value::String("a".into())));
    /// assert_eq!(document.get("db.port"), Some(&Value::Integer(5432)));
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Parse`](crate::ErrorKind::Parse) if the text is not a valid
    /// `format` document, and [`ErrorKind::Backend`](crate::ErrorKind::Backend)
    /// if this build has that format's feature off. The message is stripped the
    /// same way every other backend failure here is — the key and the kind of
    /// thing that was there, never the value.
    pub fn parse(text: &str, format: crate::Format) -> Result<Self, crate::Error> {
        crate::document::parse(text, format)
    }

    /// Merges `other` over this value: later wins, tables deep.
    ///
    /// The rule the crate already teaches for files, applied to two trees.
    /// Where both sides have a table the merge descends into it; anywhere else
    /// `other` replaces what was there, **arrays included** — a later document
    /// supplying `tags = ["b"]` means those tags and not the earlier ones, which
    /// is what every layer in this crate already means by it.
    pub fn merge(&mut self, other: Value) {
        match (self, other) {
            (Value::Table(base), Value::Table(overlay)) => {
                for (key, value) in overlay {
                    match base.entry(key) {
                        std::collections::btree_map::Entry::Occupied(mut existing) => {
                            existing.get_mut().merge(value);
                        }
                        std::collections::btree_map::Entry::Vacant(empty) => {
                            empty.insert(value);
                        }
                    }
                }
            }
            (base, overlay) => *base = overlay,
        }
    }

    /// Every leaf path both trees supply — what [`merge`](Self::merge) would
    /// silently resolve.
    ///
    /// For the caller whose documents are meant to be *disjoint*: keys read
    /// from a prefix are sections nobody intended to overlap, so an overlap
    /// there is a deployment bug worth an error rather than a merge. Paths in
    /// sorted order, and paths only — this is a diagnostic, so it names what
    /// collided and never what either side held.
    ///
    /// A path where both sides hold a table is not a collision; the tables
    /// merge. A path where either side holds an array or a scalar is.
    #[must_use]
    pub fn overlapping_paths(&self, other: &Value) -> Vec<String> {
        let mut paths = Vec::new();

        overlaps(self, other, &mut Vec::new(), &mut paths);
        paths.sort();

        paths
    }

    /// Renders this tree as the text of a `format` document.
    ///
    /// The way back out, so a merged tree can be handed to something that takes
    /// text — [`Fetched::new`](crate::Fetched::new), a file, a socket.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Backend`](crate::ErrorKind::Backend) if this build has that
    /// format's feature off, and [`ErrorKind::Type`](crate::ErrorKind::Type) if
    /// the tree is not a table — a document has named keys at its root — or
    /// holds something the format cannot express, such as a null in TOML.
    pub fn render(&self, format: crate::Format) -> Result<String, crate::Error> {
        let Value::Table(table) = self else {
            return Err(crate::Error::new(
                crate::ErrorKind::Type,
                "only a table can be a document; this tree is a scalar or a list",
            ));
        };

        crate::write::render(table, format)
    }
}

/// Every leaf path in a table, without a `Value` to hold it.
///
/// [`Value::leaf_paths`] is the public door and takes a whole value; the
/// fold has a bare table and would otherwise have to clone the resolved
/// configuration to ask this question.
pub(crate) fn leaf_paths_of(table: &BTreeMap<String, Value>) -> Vec<String> {
    let mut paths = Vec::new();
    let mut path = Vec::new();

    for (key, value) in table {
        path.push(key.clone());
        leaves(value, &mut path, &mut paths);
        path.pop();
    }

    paths
}

/// Records the dotted path of every leaf below `value`.
fn leaves(value: &Value, path: &mut Vec<String>, found: &mut Vec<String>) {
    match value {
        Value::Table(table) if !table.is_empty() => {
            for (key, nested) in table {
                path.push(key.clone());
                leaves(nested, path, found);
                path.pop();
            }
        }
        _ => found.push(path.join(".")),
    }
}

/// Records the dotted path of every leaf the two trees both supply.
fn overlaps(left: &Value, right: &Value, path: &mut Vec<String>, found: &mut Vec<String>) {
    let (Value::Table(left), Value::Table(right)) = (left, right) else {
        found.push(path.join("."));
        return;
    };

    for (key, value) in left {
        let Some(other) = right.get(key) else {
            continue;
        };

        path.push(key.clone());
        overlaps(value, other, path, found);
        path.pop();
    }
}

/// Hand-written because `f64` is not `Hash`, and because what a *fingerprint*
/// wants from a float is not what arithmetic wants: hashing through
/// [`f64::to_bits`] makes `-0.0` and `0.0` hash differently, which is right
/// here — they are different bytes in the file, and the cache's question is
/// "is this the same document", not "is this the same number".
///
/// That is also why this is deliberately *not* consistent with `PartialEq`
/// in the two places IEEE 754 is not: `-0.0 == 0.0` while their hashes
/// differ, and no `NaN` equals itself while every `NaN` payload hashes
/// stably. `Value` is not `Eq` for exactly those reasons, so there is no
/// `Hash`/`Eq` contract to break.
impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // The discriminant first, so a shape change alone moves the hash:
        // without it `Integer(1)` and a one-element table could collide
        // through their payloads.
        std::mem::discriminant(self).hash(state);

        match self {
            Self::Null => {}
            Self::Bool(value) => value.hash(state),
            Self::Integer(value) => value.hash(state),
            Self::Float(value) => value.to_bits().hash(state),
            Self::String(value) => value.hash(state),
            Self::Array(values) => values.hash(state),
            Self::Table(table) => table.hash(state),
        }
    }
}

/// The impl that makes a configuration with no struct behind it possible.
///
/// `DeserializeOwned` is the only bound the engine puts on a configuration
/// type, so this one line is what turns `Dynamic<Value>`,
/// [`Builder::values`](crate::Builder::values) and `load::<Value>` from
/// "would not compile" into the schemaless shape — with layering, watching,
/// the last-known-good cache and the reload hooks all working unchanged,
/// because none of them ever knew what `T` was.
///
/// Deliberately `deserialize_any`: a configuration value is whatever the
/// source said it was, which is the one place in serde where self-describing
/// is the right answer. The two numeric edges match the walk in from the
/// resolved tree exactly — every integer widens to `i128`, and the one
/// unrepresentable case (a `u128` above `i128::MAX`) arrives as a float —
/// so a value that reaches this type through serde and one that reaches it
/// by walking the resolved tree are the same value.
impl<'de> serde::Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(AnyValue)
    }
}

struct AnyValue;

impl<'de> serde::de::Visitor<'de> for AnyValue {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any configuration value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Value, E> {
        Ok(Value::Integer(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Value, E> {
        // Lossy above `i128::MAX`, and the same lossy the walk from figment
        // takes: a configuration value up there is measuring something no
        // unit this crate knows about.
        Ok(i128::try_from(value).map_or_else(|_| Value::Float(value as f64), Value::Integer))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(Value::Float(value))
    }

    fn visit_char<E>(self, value: char) -> Result<Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_newtype_struct<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));

        while let Some(value) = seq.next_element()? {
            values.push(value);
        }

        Ok(Value::Array(values))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut table = BTreeMap::new();

        // Keys are strings because configuration keys are: every format this
        // crate reads spells them that way, and a non-string key here is a
        // caller deserializing something that is not a configuration.
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            table.insert(key, value);
        }

        Ok(Value::Table(table))
    }
}

/// The way back out through serde, for [`crate::save`] and
/// [`crate::changed_paths`] — the two surfaces that take `T: Serialize` and
/// would otherwise be the only ones a schemaless configuration could not
/// reach.
///
/// Integers narrow exactly as the walk back out narrows them, and for the
/// same reason: this type widens every integer on the way in so the boundary
/// needs no sign decision, while a serializer does — `toml` refuses an
/// `i128` whatever the number in it is.
///
/// This is *handover*, like [`crate::Snapshot::to_value`] and unlike
/// [`Debug`]: it emits real values, secrets included, because that is what
/// serializing a configuration means. The paths-only rule governs what this
/// crate prints, not what a caller asks it to write.
impl serde::Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};

        match self {
            // `None` rather than `Unit`, and the difference is the writers':
            // a format with no unit type refuses `Unit` outright, where
            // `None` is the absent key it is meant to be. Every format this
            // crate reads back renders the two the same way, so nothing but
            // TOML can tell the difference.
            Value::Null => serializer.serialize_none(),
            Value::Bool(boolean) => serializer.serialize_bool(*boolean),
            Value::Integer(number) => match (i64::try_from(*number), u64::try_from(*number)) {
                (Ok(signed), _) => serializer.serialize_i64(signed),
                (_, Ok(unsigned)) => serializer.serialize_u64(unsigned),
                _ => serializer.serialize_i128(*number),
            },
            Value::Float(number) => serializer.serialize_f64(*number),
            Value::String(text) => serializer.serialize_str(text),
            Value::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;

                for value in values {
                    sequence.serialize_element(value)?;
                }

                sequence.end()
            }
            Value::Table(table) => {
                let mut map = serializer.serialize_map(Some(table.len()))?;

                for (key, value) in table {
                    map.serialize_entry(key, value)?;
                }

                map.end()
            }
        }
    }
}

impl std::fmt::Debug for Value {
    /// Shape and keys, never values — the line every diagnostic in this
    /// crate holds, held here too because `to_value` hands over the same
    /// secret-bearing data `Snapshot` guards.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => f.write_str("Null"),
            Self::Bool(_) => f.write_str("Bool(***)"),
            Self::Integer(_) => f.write_str("Integer(***)"),
            Self::Float(_) => f.write_str("Float(***)"),
            Self::String(_) => f.write_str("String(***)"),
            Self::Array(values) => f.debug_list().entries(values.iter()).finish(),
            Self::Table(table) => f.debug_map().entries(table.iter()).finish(),
        }
    }
}

/// The walk from figment's tree, tags dropped, no serialization involved.
/// The obvious conversions, so a tree can be written down in code.
///
/// A configuration is usually read rather than built, but the places that
/// build one — a test, a default, a binding handing values back — should not
/// have to name the variant every time.
macro_rules! from_integer {
    ($($type:ty),* $(,)?) => {$(
        impl From<$type> for Value {
            fn from(number: $type) -> Self {
                Value::Integer(i128::from(number))
            }
        }
    )*};
}

from_integer!(u8, u16, u32, u64, i8, i16, i32, i64, i128);

impl From<bool> for Value {
    fn from(boolean: bool) -> Self {
        Value::Bool(boolean)
    }
}

impl From<f64> for Value {
    fn from(number: f64) -> Self {
        Value::Float(number)
    }
}

impl From<f32> for Value {
    fn from(number: f32) -> Self {
        Value::Float(f64::from(number))
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Value::String(text.to_owned())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Value::String(text)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(values: Vec<T>) -> Self {
        Value::Array(values.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<std::collections::BTreeMap<String, T>> for Value {
    fn from(table: std::collections::BTreeMap<String, T>) -> Self {
        Value::Table(
            table
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
        )
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map_or(Value::Null, Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::figment::{from_figment, to_figment};

    #[test]
    fn the_walk_preserves_shape_and_numbers() {
        let source: figment::value::Value = figment::value::Value::serialize(serde_json::json!({
            "port": 5432,
            "ratio": 0.5,
            "tls": true,
            "host": "db",
            "tags": ["a", "b"],
            "pool": { "max": 8 },
        }))
        .expect("a literal serializes");

        let value = from_figment(&source);

        assert_eq!(value.get("port"), Some(&Value::Integer(5432)));
        assert_eq!(value.get("ratio"), Some(&Value::Float(0.5)));
        assert_eq!(value.get("tls"), Some(&Value::Bool(true)));
        assert_eq!(value.get("host"), Some(&Value::String("db".into())));
        assert_eq!(value.get("pool.max"), Some(&Value::Integer(8)));
        assert_eq!(
            value.get("tags"),
            Some(&Value::Array(vec![
                Value::String("a".into()),
                Value::String("b".into())
            ]))
        );
    }

    fn hash_of(value: &Value) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);

        hasher.finish()
    }

    /// The cache's identity is a hash of this tree, so a value that is the
    /// same document has to hash the same however it was assembled...
    #[test]
    fn an_equal_tree_hashes_equal() {
        let one = Value::Table(BTreeMap::from([
            ("host".to_owned(), Value::String("db".to_owned())),
            ("port".to_owned(), Value::Integer(5432)),
        ]));
        let two = one.clone();

        assert_eq!(hash_of(&one), hash_of(&two));
    }

    /// ...and a different document has to hash differently, including the
    /// two cases a numeric comparison would call equal.
    #[test]
    fn a_signed_zero_is_a_different_document() {
        assert_eq!(Value::Float(-0.0), Value::Float(0.0), "as numbers");
        assert_ne!(
            hash_of(&Value::Float(-0.0)),
            hash_of(&Value::Float(0.0)),
            "as bytes in a file, which is what a fingerprint answers for"
        );
    }

    #[test]
    fn a_whole_number_is_not_the_float_that_prints_the_same() {
        assert_ne!(Value::Integer(1), Value::Float(1.0));
        assert_ne!(hash_of(&Value::Integer(1)), hash_of(&Value::Float(1.0)));
    }

    /// The walk out narrows, so the walk back in has to widen to the same
    /// number — otherwise a round trip through the seam quietly changes the
    /// document, which the cache's fingerprint would then call a reload.
    #[test]
    fn the_walk_back_narrows_without_changing_the_number() {
        for number in [
            0,
            1,
            -1,
            i128::from(i64::MIN),
            i128::from(u64::MAX),
            i128::MAX,
        ] {
            assert_eq!(
                from_figment(&to_figment(&Value::Integer(number))),
                Value::Integer(number),
                "{number}"
            );
        }
    }

    #[test]
    fn the_walk_back_preserves_every_shape() {
        let tree = Value::Table(BTreeMap::from([
            ("null".to_owned(), Value::Null),
            ("bool".to_owned(), Value::Bool(true)),
            ("float".to_owned(), Value::Float(0.5)),
            ("text".to_owned(), Value::String("a".to_owned())),
            (
                "list".to_owned(),
                Value::Array(vec![Value::Integer(1), Value::Null]),
            ),
            (
                "table".to_owned(),
                Value::Table(BTreeMap::from([("nested".to_owned(), Value::Integer(2))])),
            ),
        ]));

        assert_eq!(from_figment(&to_figment(&tree)), tree);
    }

    /// Reachable by hand, so it says which feature is missing rather than
    /// failing in a way that reads like a malformed document.
    #[cfg(not(any(feature = "json", feature = "toml", feature = "yaml")))]
    #[test]
    fn a_format_this_build_cannot_read_names_its_feature() {
        let error = Value::parse("{}", crate::Format::Json).expect_err("no format is enabled");

        assert_eq!(error.kind(), crate::ErrorKind::Backend);
        assert!(error.message().contains("json"), "{error}");

        let error = Value::Table(BTreeMap::new())
            .render(crate::Format::Json)
            .expect_err("no format is enabled");

        assert_eq!(error.kind(), crate::ErrorKind::Backend);
    }

    /// The `Deserialize` impl is what makes `Dynamic<Value>` compile, so the
    /// property that matters is that it agrees with the walk: a value that
    /// arrives through serde and one that arrives by walking the resolved
    /// tree must be the same value, or the schemaless configuration and the
    /// exported one disagree about what was in the file.
    #[test]
    fn the_serde_road_and_the_walk_agree() {
        let source: figment::value::Value = figment::value::Value::serialize(serde_json::json!({
            "port": 5432,
            "ratio": 0.5,
            "tls": true,
            "host": "db",
            "nothing": (),
            "tags": ["a", { "nested": 1 }],
            "pool": { "max": 8, "empty": {} },
        }))
        .expect("a literal serializes");

        assert_eq!(
            from_figment(&source),
            source.deserialize::<Value>().expect("any value is a Value"),
        );
    }

    /// The two numeric edges the walk documents, held on the serde road too.
    #[test]
    fn the_serde_road_widens_and_gives_up_at_the_same_places() {
        use serde::de::value::{Error, I128Deserializer, U128Deserializer, UnitDeserializer};
        use serde::Deserialize as _;

        let signed = |number| Value::deserialize(I128Deserializer::<Error>::new(number));
        let unsigned = |number| Value::deserialize(U128Deserializer::<Error>::new(number));

        assert_eq!(signed(i128::MIN).unwrap(), Value::Integer(i128::MIN));
        assert_eq!(
            unsigned(u128::try_from(i128::MAX).expect("in range")).unwrap(),
            Value::Integer(i128::MAX)
        );
        assert_eq!(
            unsigned(u128::MAX).unwrap(),
            Value::Float(u128::MAX as f64),
            "the one unrepresentable case arrives lossily, as the walk does it"
        );

        // A document is a table, but a value below one need not be.
        assert_eq!(
            Value::deserialize(UnitDeserializer::<Error>::new()).unwrap(),
            Value::Null
        );
    }

    /// Serializing narrows exactly as the walk out does, so a tree that went
    /// through serde survives a format with one integer type.
    #[test]
    fn serializing_narrows_the_way_the_walk_out_narrows() {
        for number in [0, 1, -1, i128::from(i64::MIN), i128::from(u64::MAX)] {
            let rendered =
                serde_json::to_string(&Value::Integer(number)).expect("a number serializes");

            assert_eq!(rendered, number.to_string());
        }

        let tree = Value::Table(BTreeMap::from([
            ("null".to_owned(), Value::Null),
            ("ratio".to_owned(), Value::Float(0.5)),
            (
                "tags".to_owned(),
                Value::Array(vec![Value::String("a".to_owned())]),
            ),
        ]));

        assert_eq!(
            serde_json::to_value(&tree)
                .expect("a tree serializes")
                .to_string(),
            r#"{"null":null,"ratio":0.5,"tags":["a"]}"#
        );

        // And back: the round trip through serde is the identity, which is
        // what `save` + a reload amounts to.
        assert_eq!(
            serde_json::from_str::<Value>(&serde_json::to_string(&tree).unwrap()).unwrap(),
            tree
        );
    }

    #[test]
    fn a_typed_read_reports_the_path_and_the_kind_that_was_there() {
        let tree = Value::Table(BTreeMap::from([(
            "pool".to_owned(),
            Value::Table(BTreeMap::from([(
                "max".to_owned(),
                Value::String("not-a-number".to_owned()),
            )])),
        )]));

        assert_eq!(
            tree.get_as::<u16>("pool.max").unwrap_err().kind(),
            crate::ErrorKind::Type
        );
        assert_eq!(
            tree.get_as::<u16>("pool.max").unwrap_err().path(),
            "pool.max"
        );
        assert_eq!(
            tree.get_as::<u16>("pool.min").unwrap_err().kind(),
            crate::ErrorKind::Missing
        );
        assert_eq!(tree.get_as::<String>("pool.max").unwrap(), "not-a-number");
    }

    #[test]
    fn leaf_paths_stops_at_arrays_and_keeps_empty_tables() {
        let tree = Value::Table(BTreeMap::from([
            ("host".to_owned(), Value::String("db".to_owned())),
            ("empty".to_owned(), Value::Table(BTreeMap::new())),
            (
                "tags".to_owned(),
                Value::Array(vec![Value::Integer(1), Value::Integer(2)]),
            ),
            (
                "pool".to_owned(),
                Value::Table(BTreeMap::from([("max".to_owned(), Value::Integer(8))])),
            ),
        ]));

        assert_eq!(tree.leaf_paths(), ["empty", "host", "pool.max", "tags"]);
    }

    #[test]
    fn a_step_through_a_leaf_is_none_and_the_empty_path_is_identity() {
        let value = Value::Table(BTreeMap::from([("port".to_owned(), Value::Integer(1))]));

        assert_eq!(value.get("port.deeper"), None);
        assert_eq!(value.get("missing"), None);
        assert_eq!(value.get(""), Some(&value));
    }
}

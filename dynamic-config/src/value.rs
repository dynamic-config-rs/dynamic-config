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

use std::collections::BTreeMap;

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
pub(crate) fn from_figment(value: &figment::value::Value) -> Value {
    use figment::value::{Empty, Num};

    match value {
        figment::value::Value::String(_, string) => Value::String(string.clone()),
        figment::value::Value::Char(_, character) => Value::String(character.to_string()),
        figment::value::Value::Bool(_, boolean) => Value::Bool(*boolean),
        figment::value::Value::Num(_, number) => match number {
            Num::U8(n) => Value::Integer(i128::from(*n)),
            Num::U16(n) => Value::Integer(i128::from(*n)),
            Num::U32(n) => Value::Integer(i128::from(*n)),
            Num::U64(n) => Value::Integer(i128::from(*n)),
            Num::USize(n) => Value::Integer(*n as i128),
            Num::U128(n) => i128::try_from(*n)
                .map(Value::Integer)
                .unwrap_or(Value::Float(*n as f64)),
            Num::I8(n) => Value::Integer(i128::from(*n)),
            Num::I16(n) => Value::Integer(i128::from(*n)),
            Num::I32(n) => Value::Integer(i128::from(*n)),
            Num::I64(n) => Value::Integer(i128::from(*n)),
            Num::ISize(n) => Value::Integer(*n as i128),
            Num::I128(n) => Value::Integer(*n),
            Num::F32(n) => Value::Float(f64::from(*n)),
            Num::F64(n) => Value::Float(*n),
        },
        figment::value::Value::Empty(_, Empty::None | Empty::Unit) => Value::Null,
        figment::value::Value::Dict(_, dict) => Value::Table(
            dict.iter()
                .map(|(key, value)| (key.clone(), from_figment(value)))
                .collect(),
        ),
        figment::value::Value::Array(_, values) => {
            Value::Array(values.iter().map(from_figment).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_step_through_a_leaf_is_none_and_the_empty_path_is_identity() {
        let value = Value::Table(BTreeMap::from([("port".to_owned(), Value::Integer(1))]));

        assert_eq!(value.get("port.deeper"), None);
        assert_eq!(value.get("missing"), None);
        assert_eq!(value.get(""), Some(&value));
    }
}

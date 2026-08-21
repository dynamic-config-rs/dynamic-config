//! Between the `config` crate's value tree and this crate's.
//!
//! One match over the backend's kinds, in one place, because two callers
//! need it and only one of them is recording provenance: the fold walks a
//! merged tree writing down who won each leaf, and the reader walks a
//! freshly parsed document where there is nothing to write down yet.

use crate::value::Value;

/// One of the backend's values, as one of this crate's.
fn leaf(value: &config_rs::Value) -> Value {
    match &value.kind {
        config_rs::ValueKind::Nil => Value::Null,
        config_rs::ValueKind::Boolean(boolean) => Value::Bool(*boolean),
        config_rs::ValueKind::I64(number) => Value::Integer(i128::from(*number)),
        config_rs::ValueKind::I128(number) => Value::Integer(*number),
        config_rs::ValueKind::U64(number) => Value::Integer(i128::from(*number)),
        config_rs::ValueKind::U128(number) => {
            i128::try_from(*number).map_or_else(|_| Value::Float(*number as f64), Value::Integer)
        }
        config_rs::ValueKind::Float(number) => Value::Float(*number),
        config_rs::ValueKind::String(text) => Value::String(text.clone()),
        config_rs::ValueKind::Array(values) => {
            Value::Array(values.iter().map(from_config_rs).collect())
        }
        config_rs::ValueKind::Table(table) => Value::Table(
            table
                .iter()
                .map(|(key, value)| (key.clone(), from_config_rs(value)))
                .collect(),
        ),
    }
}

/// The backend's tree as this crate's, with no provenance to record.
///
/// What a reader hands back: a document that has just been parsed has
/// exactly one source, and the layer it belongs to is the caller's to
/// know.
pub(crate) fn from_config_rs(value: &config_rs::Value) -> Value {
    leaf(value)
}

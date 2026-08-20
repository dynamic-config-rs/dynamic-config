//! Between figment's value tree and this crate's.
//!
//! Compiled under `test` as well as under the feature: figment is a
//! permanent dev-dependency, because every piece this crate ported out of
//! it — the value-string reader, the deserializer, the serializer, the
//! fold — is proved against it, and those comparisons need these two
//! functions whether or not a user asked for the feature.

use crate::value::Value;

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

/// The walk back, for [`Value::render`]: this crate's serializers all take
/// figment's tree, and one of them is what [`crate::save`] already writes with.
///
/// Every value is tagged [`Tag::Default`](figment::value::Tag::Default) —
/// a tag records which provider supplied a value, and a tree assembled by a
/// caller was supplied by none of them.
pub(crate) fn to_figment(value: &Value) -> figment::value::Value {
    use figment::value::{Empty, Num, Tag};

    match value {
        // `None` rather than `Unit`, and the difference is the writers': a
        // format with no unit type refuses `Unit` outright, where `None` is
        // the absent key it is meant to be. Readers want the other one, and
        // ask for it where they read.
        Value::Null => figment::value::Value::Empty(Tag::Default, Empty::None),
        Value::Bool(boolean) => figment::value::Value::Bool(Tag::Default, *boolean),
        // Narrowed rather than emitted as `I128`: `Value` widens every integer
        // on the way in so the boundary needs no sign decision, but a
        // serializer does — `toml` refuses an `i128` outright, whatever the
        // number in it is. Signed first, so a round trip through a format that
        // has one integer type comes back the width it went in as.
        Value::Integer(number) => figment::value::Value::Num(
            Tag::Default,
            i64::try_from(*number).map_or_else(
                |_| u64::try_from(*number).map_or(Num::I128(*number), Num::U64),
                Num::I64,
            ),
        ),
        Value::Float(number) => figment::value::Value::Num(Tag::Default, Num::F64(*number)),
        Value::String(text) => figment::value::Value::String(Tag::Default, text.clone()),
        Value::Array(values) => {
            figment::value::Value::Array(Tag::Default, values.iter().map(to_figment).collect())
        }
        Value::Table(table) => figment::value::Value::Dict(
            Tag::Default,
            table
                .iter()
                .map(|(key, value)| (key.clone(), to_figment(value)))
                .collect(),
        ),
    }
}

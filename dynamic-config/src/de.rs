//! Reading a resolved tree into the caller's type.
//!
//! The last step of every load: the layers are merged, the winner of each
//! path is known, and what remains is handing that tree to `serde`. This
//! module is that hand-off — a [`serde::Deserializer`] over [`Value`] — and
//! it is deliberately strict: a string stays a string, a number stays a
//! number, and the widening that turns `"8080"` into a port happened
//! earlier, in [`crate::text_value`], where the text arrived.
//!
//! Keeping those two apart is what makes the rule explainable: **text
//! sources are read loosely, documents are read exactly.** `PORT=8080` is a
//! number because the environment carries no types; `port = "8080"` in a
//! TOML file is a string because the file said so, and a `u16` field
//! refuses it.
//!
//! Errors carry the path they stopped at, built up as the walk unwinds, so
//! "invalid type: string" arrives as `database.port: invalid type: string`.

use std::fmt;

use serde::de::{self, Deserializer, IntoDeserializer, MapAccess, SeqAccess, Visitor};

use crate::value::Value;

/// What stopped a deserialization, and where.
///
/// The path is built key by key as the error unwinds — the visitor that
/// failed knows what went wrong, and every frame above it knows one more
/// segment of where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Error {
    kind: crate::error::ErrorKind,
    message: String,
    path: Vec<String>,
}

impl Error {
    /// One more segment on the front: the caller is a frame above.
    fn prefixed(mut self, key: &str) -> Self {
        self.path.insert(0, key.to_owned());

        self
    }

    /// The dotted path, empty when the failure was at the root.
    pub(crate) fn path(&self) -> String {
        self.path.join(".")
    }

    /// This crate's error, with the path it stopped at as the key.
    pub(crate) fn into_error(self) -> crate::Error {
        let error = crate::Error::new(self.kind, self.message.clone());

        if self.path.is_empty() {
            error
        } else {
            error.prepend_key(self.path())
        }
    }

    fn of(kind: crate::error::ErrorKind, message: String) -> Self {
        Error {
            kind,
            message,
            path: Vec::new(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            formatter.write_str(&self.message)
        } else {
            write!(formatter, "{}: {}", self.path(), self.message)
        }
    }
}

impl std::error::Error for Error {}

/// `serde`'s own vocabulary is implemented rather than left to default,
/// for two reasons. The kind survives — a missing field is
/// [`ErrorKind::Missing`] because `serde` said so, not because a message
/// started with the word "missing". And **no value reaches a message**:
/// `serde`'s default rendering of a type mismatch includes the offending
/// value, which for `password = 12` is exactly the leak this crate is built
/// to prevent, so every unexpected thing is named by its kind instead.
impl de::Error for Error {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Error::of(crate::error::ErrorKind::Type, message.to_string())
    }

    fn invalid_type(unexpected: de::Unexpected<'_>, expected: &dyn de::Expected) -> Self {
        Error::of(
            crate::error::ErrorKind::Type,
            format!("invalid type: {}, expected {expected}", kind_of(unexpected)),
        )
    }

    fn invalid_value(unexpected: de::Unexpected<'_>, expected: &dyn de::Expected) -> Self {
        Error::of(
            crate::error::ErrorKind::Type,
            format!(
                "invalid value: {}, expected {expected}",
                kind_of(unexpected)
            ),
        )
    }

    fn invalid_length(length: usize, expected: &dyn de::Expected) -> Self {
        Error::of(
            crate::error::ErrorKind::Type,
            format!("invalid length {length}, expected {expected}"),
        )
    }

    // The only structural error that names its own key: the visitor knows
    // which field it did not find, and nothing above it will, because the
    // frame that would have prefixed the path is the one that never ran.
    fn missing_field(field: &'static str) -> Self {
        Error::of(
            crate::error::ErrorKind::Missing,
            format!("missing field `{field}`"),
        )
        .prefixed(field)
    }

    fn unknown_field(field: &str, expected: &'static [&'static str]) -> Self {
        Error::of(
            crate::error::ErrorKind::Type,
            format!("unknown field `{field}`, expected one of {expected:?}"),
        )
    }

    fn duplicate_field(field: &'static str) -> Self {
        Error::of(
            crate::error::ErrorKind::Type,
            format!("duplicate field `{field}`"),
        )
    }

    fn unknown_variant(variant: &str, expected: &'static [&'static str]) -> Self {
        Error::of(
            crate::error::ErrorKind::Type,
            format!("unknown variant `{variant}`, expected one of {expected:?}"),
        )
    }
}

/// What was there, named by its kind and never by its value.
///
/// The same vocabulary the loader has always used for the backend's own
/// mismatches, so a message reads the same whichever door produced it.
fn kind_of(unexpected: de::Unexpected<'_>) -> &'static str {
    use de::Unexpected;

    match unexpected {
        Unexpected::Bool(_) => "a boolean",
        Unexpected::Unsigned(_) => "an unsigned integer",
        Unexpected::Signed(_) => "a signed integer",
        Unexpected::Float(_) => "a float",
        Unexpected::Char(_) => "a character",
        Unexpected::Str(_) => "a string",
        Unexpected::Bytes(_) => "a byte string",
        Unexpected::Unit => "a unit",
        Unexpected::Option => "an option",
        Unexpected::NewtypeStruct => "a newtype struct",
        Unexpected::Seq => "a list",
        Unexpected::Map => "a table",
        Unexpected::Enum => "an enum",
        Unexpected::UnitVariant => "a unit variant",
        Unexpected::NewtypeVariant => "a newtype variant",
        Unexpected::TupleVariant => "a tuple variant",
        Unexpected::StructVariant => "a struct variant",
        // Free-form, and from whatever produced the error, so not ours to
        // vouch for.
        Unexpected::Other(_) => "something else",
    }
}

type Result<T> = std::result::Result<T, Error>;

/// A tree, ready to be read into a type.
///
/// A wrapper rather than an implementation on [`Value`] itself: the trait's
/// error type would have to be public with it, and this crate already has
/// one error type too many for that to be an improvement.
pub(crate) struct Reader<'a>(pub(crate) &'a Value);

impl<'de> Deserializer<'de> for Reader<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(boolean) => visitor.visit_bool(*boolean),
            Value::Integer(number) => integer(*number, visitor),
            Value::Float(number) => visitor.visit_f64(*number),
            Value::String(text) => visitor.visit_str(text),
            Value::Array(values) => visitor.visit_seq(Sequence {
                values: values.iter().enumerate(),
                remaining: values.len(),
            }),
            Value::Table(table) => visitor.visit_map(Table {
                entries: table.iter(),
                value: None,
            }),
        }
    }

    /// A null is a `None`; everything else is a `Some` of itself.
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    /// The three shapes an enum arrives in: a name, a single-entry table
    /// (`{ variant = payload }`), and an index.
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        use serde::de::value::MapAccessDeserializer;

        match self.0 {
            Value::String(text) => visitor.visit_enum(text.as_str().into_deserializer()),
            Value::Table(table) => visitor.visit_enum(MapAccessDeserializer::new(Table {
                entries: table.iter(),
                value: None,
            })),
            Value::Integer(number) => match u32::try_from(*number) {
                Ok(index) => visitor.visit_enum(index.into_deserializer()),
                Err(_) => self.deserialize_any(visitor),
            },
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    /// Configuration is text people write and read; nothing here is a
    /// compact wire format.
    fn is_human_readable(&self) -> bool {
        true
    }

    serde::forward_to_deserialize_any! {
        bool u8 u16 u32 u64 i8 i16 i32 i64 f32 f64 char str string seq bytes
        byte_buf map unit struct ignored_any unit_struct tuple_struct tuple
        identifier
    }
}

/// An integer, offered at the narrowest width that holds it.
///
/// The tree keeps one integer type, so the width a source happened to use is
/// gone by the time this runs. Non-negative numbers are offered unsigned,
/// which is the width every text source produced before the widths were
/// erased, and negative ones signed. A visitor that accepts either — every
/// integer `serde` derives does — cannot tell the difference; one that
/// accepts only one of them gets the one its values could have come from.
fn integer<'de, V: Visitor<'de>>(number: i128, visitor: V) -> Result<V::Value> {
    if let Ok(unsigned) = u64::try_from(number) {
        return visitor.visit_u64(unsigned);
    }

    if let Ok(signed) = i64::try_from(number) {
        return visitor.visit_i64(signed);
    }

    visitor.visit_i128(number)
}

/// A table, walked in key order — the tree is sorted, so the walk is stable.
struct Table<'a> {
    entries: std::collections::btree_map::Iter<'a, String, Value>,
    value: Option<(&'a str, &'a Value)>,
}

impl<'de> MapAccess<'de> for Table<'de> {
    type Error = Error;

    fn next_key_seed<K: de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };

        self.value = Some((key, value));

        seed.deserialize(key.as_str().into_deserializer())
            .map(Some)
            .map_err(|error: Error| error.prefixed(key))
    }

    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        let (key, value) = self
            .value
            .take()
            .expect("a value is only asked for after its key");

        seed.deserialize(Reader(value))
            .map_err(|error: Error| error.prefixed(key))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

/// An array, where the "key" in a path is the index.
struct Sequence<'a> {
    values: std::iter::Enumerate<std::slice::Iter<'a, Value>>,
    remaining: usize,
}

impl<'de> SeqAccess<'de> for Sequence<'de> {
    type Error = Error;

    fn next_element_seed<T: de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>> {
        let Some((index, value)) = self.values.next() else {
            return Ok(None);
        };

        self.remaining -= 1;

        seed.deserialize(Reader(value))
            .map(Some)
            .map_err(|error: Error| error.prefixed(&index.to_string()))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::IpAddr;
    use std::path::PathBuf;

    use proptest::prelude::*;
    use serde::Deserialize;

    use super::Reader;
    use crate::value::Value;

    /// What this module answers.
    fn ours<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
        T::deserialize(Reader(value)).map_err(|error| error.path())
    }

    /// What the implementation this one was ported from answers, given the
    /// same tree. Compared on the outcome and on the path an error stopped
    /// at — the two things a caller can observe. The wording of a message is
    /// this crate's own and deliberately not pinned to the original's.
    ///
    /// The path is read the way this crate has always read it: the original
    /// files a missing field under its kind rather than its path, and the
    /// translation has filled the path in from the kind since the first
    /// release. Comparing against the bare path would hold the port to a
    /// behaviour no caller ever saw.
    fn original<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
        T::deserialize(&as_a_provider_would(value)).map_err(|error: figment::Error| {
            crate::backend::figment::error::error_path(&error).join(".")
        })
    }

    /// The tree a provider hands over, which is what the original is asked
    /// about.
    ///
    /// `to_figment` is the *writers'* conversion and renders a null as an
    /// absent key, because a format with no unit type refuses anything else.
    /// A reader is given the null the document actually held, and the two
    /// deserialize differently — so the comparison uses the reader's shape.
    fn as_a_provider_would(value: &Value) -> figment::value::Value {
        use figment::value::{Empty, Tag};

        match value {
            Value::Null => figment::value::Value::Empty(Tag::Default, Empty::Unit),
            Value::Array(values) => figment::value::Value::Array(
                Tag::Default,
                values.iter().map(as_a_provider_would).collect(),
            ),
            Value::Table(table) => figment::value::Value::Dict(
                Tag::Default,
                table
                    .iter()
                    .map(|(key, value)| (key.clone(), as_a_provider_would(value)))
                    .collect(),
            ),
            other => crate::backend::figment::to_figment(other),
        }
    }

    /// Every target type the comparison runs, so a divergence in one shape
    /// cannot hide behind another's agreement.
    fn agrees(value: &Value) -> Result<(), String> {
        macro_rules! compare {
            ($($type:ty),* $(,)?) => {$(
                if ours::<$type>(value) != original::<$type>(value) {
                    return Err(format!(
                        "{} read {:?} as {:?}, the original as {:?}",
                        stringify!($type),
                        value,
                        ours::<$type>(value),
                        original::<$type>(value),
                    ));
                }
            )*};
        }

        #[derive(Debug, PartialEq, Deserialize)]
        struct Nested {
            host: String,
            port: u16,
            #[serde(default)]
            tags: Vec<String>,
        }

        #[derive(Debug, PartialEq, Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum Mode {
            Fast,
            Slow,
        }

        compare!(
            bool,
            u8,
            u16,
            u64,
            i8,
            i64,
            i128,
            f64,
            char,
            String,
            Option<u16>,
            Option<String>,
            Vec<String>,
            Vec<u16>,
            BTreeMap<String, u32>,
            (u16, String),
            IpAddr,
            PathBuf,
            Mode,
            Nested,
            serde_json::Value,
        );

        Ok(())
    }

    /// The shapes a resolved tree actually takes.
    fn trees() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(|number| Value::Integer(i128::from(number))),
            any::<i128>().prop_map(Value::Integer),
            any::<f64>().prop_map(Value::Float),
            "[a-z0-9.:_-]{0,12}".prop_map(Value::String),
            // The strings that mean something to a typed field, so the
            // strict-document rule is exercised rather than assumed.
            prop_oneof![
                Just("8080".to_owned()),
                Just("true".to_owned()),
                Just("127.0.0.1".to_owned()),
                Just("fast".to_owned()),
                Just("/etc/app".to_owned()),
            ]
            .prop_map(Value::String),
        ];

        leaf.prop_recursive(4, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                prop::collection::btree_map("[a-z]{1,6}|host|port|tags", inner, 0..4)
                    .prop_map(Value::Table),
            ]
        })
    }

    proptest! {
        #[test]
        fn every_tree_reads_the_same(value in trees()) {
            if let Err(divergence) = agrees(&value) {
                return Err(TestCaseError::fail(divergence));
            }
        }
    }

    /// The corners worth naming, so a failure points at a case rather than
    /// at a generated tree.
    #[test]
    fn the_shapes_a_configuration_takes_read_the_same() {
        let table = |pairs: &[(&str, Value)]| {
            Value::Table(
                pairs
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), value.clone()))
                    .collect(),
            )
        };

        let cases = [
            Value::Null,
            Value::Bool(true),
            Value::Integer(8080),
            Value::Integer(-1),
            Value::Integer(i128::from(u64::MAX)),
            Value::Integer(i128::MAX),
            Value::Float(1.5),
            Value::String("8080".to_owned()),
            Value::String("fast".to_owned()),
            Value::Array(vec![Value::Integer(1), Value::Integer(2)]),
            Value::Array(vec![Value::String("a".to_owned())]),
            table(&[
                ("host", Value::String("localhost".to_owned())),
                ("port", Value::Integer(5432)),
            ]),
            // A struct field that is the wrong type: both must refuse, and
            // both must say the same path.
            table(&[
                ("host", Value::String("localhost".to_owned())),
                ("port", Value::String("not a port".to_owned())),
            ]),
            // A missing field, refused at the root.
            table(&[("host", Value::String("localhost".to_owned()))]),
            // An enum as a single-entry table.
            table(&[("fast", Value::Null)]),
        ];

        for case in cases {
            agrees(&case).expect("the reading moved away from the original");
        }
    }
}

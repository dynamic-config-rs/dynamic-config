//! Turning a program's own value into this crate's tree.
//!
//! The direction [`de`](crate::de) does not: a `T: Serialize` — a default, an
//! override, a struct on its way to a file — becomes a [`Value`]. One
//! serializer, so every door that takes a typed value (`set`, `save`, the
//! cache) agrees about what a `u8` or a `None` turns into.
//!
//! **A message never carries a value.** The only thing that can fail here is
//! a shape — a map keyed by something that is not a string — and the refusal
//! names the shape, never what was in it.

use std::collections::BTreeMap;
use std::fmt;

use serde::{ser, Serialize};

use crate::value::Value;

/// `value`, as this crate's tree.
///
/// # Errors
///
/// If the value cannot be a configuration value: a map keyed by something
/// other than a string, or a `Serialize` implementation that fails.
pub(crate) fn to_value<T: Serialize + ?Sized>(value: &T) -> std::result::Result<Value, Error> {
    value.serialize(Serializer)
}

/// What stopped a serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Error(message.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

/// The serializer itself, which holds nothing: every value is built on the
/// way out rather than accumulated in the type.
struct Serializer;

impl ser::Serializer for Serializer {
    type Ok = Value;
    type Error = Error;

    type SerializeSeq = Sequence;
    type SerializeTuple = Sequence;
    type SerializeTupleStruct = Sequence;
    type SerializeTupleVariant = Sequence;
    type SerializeMap = Table;
    type SerializeStruct = Table;
    type SerializeStructVariant = Table;

    fn serialize_bool(self, value: bool) -> Result<Value> {
        Ok(Value::Bool(value))
    }

    fn serialize_i8(self, value: i8) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn serialize_i16(self, value: i16) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn serialize_i32(self, value: i32) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn serialize_i64(self, value: i64) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn serialize_i128(self, value: i128) -> Result<Value> {
        Ok(Value::Integer(value))
    }

    fn serialize_u8(self, value: u8) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn serialize_u16(self, value: u16) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn serialize_u32(self, value: u32) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    fn serialize_u64(self, value: u64) -> Result<Value> {
        Ok(Value::Integer(i128::from(value)))
    }

    /// The one integer that does not always fit: a `u128` past `i128::MAX`
    /// keeps its magnitude and loses its exactness, which is what every
    /// configuration format this crate writes would have done to it anyway.
    fn serialize_u128(self, value: u128) -> Result<Value> {
        Ok(i128::try_from(value)
            .map(Value::Integer)
            .unwrap_or_else(|_| Value::Float(value as f64)))
    }

    fn serialize_f32(self, value: f32) -> Result<Value> {
        Ok(Value::Float(f64::from(value)))
    }

    fn serialize_f64(self, value: f64) -> Result<Value> {
        Ok(Value::Float(value))
    }

    /// A tree has no character type: a `char` is a one-character string, and
    /// reading it back asks for a `char` again.
    fn serialize_char(self, value: char) -> Result<Value> {
        Ok(Value::String(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> Result<Value> {
        Ok(Value::String(value.to_owned()))
    }

    /// Bytes are a list of numbers, which is the only shape a configuration
    /// document has for them.
    fn serialize_bytes(self, value: &[u8]) -> Result<Value> {
        Ok(Value::Array(
            value
                .iter()
                .map(|byte| Value::Integer(i128::from(*byte)))
                .collect(),
        ))
    }

    fn serialize_none(self) -> Result<Value> {
        Ok(Value::Null)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Value> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value> {
        Ok(Value::Null)
    }

    /// A fieldless variant is its own name, so `Mode::Fast` writes as
    /// `"fast"` and reads back through the same spelling.
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Value> {
        Ok(Value::String(variant.to_owned()))
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Value> {
        value.serialize(self)
    }

    /// Every other variant is a table of one key: the variant's name over
    /// what it carries.
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value> {
        Ok(tagged(variant, value.serialize(self)?))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Sequence> {
        Ok(Sequence::new(None, len))
    }

    fn serialize_tuple(self, len: usize) -> Result<Sequence> {
        Ok(Sequence::new(None, Some(len)))
    }

    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<Sequence> {
        Ok(Sequence::new(None, Some(len)))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Sequence> {
        Ok(Sequence::new(Some(variant), Some(len)))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Table> {
        Ok(Table::new(None))
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Table> {
        Ok(Table::new(None))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Table> {
        Ok(Table::new(Some(variant)))
    }
}

/// A variant's name over what it holds.
fn tagged(variant: &'static str, value: Value) -> Value {
    Value::Table(BTreeMap::from([(variant.to_owned(), value)]))
}

/// A list being built, and the variant name it goes under if it has one.
struct Sequence {
    tag: Option<&'static str>,
    values: Vec<Value>,
}

impl Sequence {
    fn new(tag: Option<&'static str>, len: Option<usize>) -> Self {
        Self {
            tag,
            values: len.map(Vec::with_capacity).unwrap_or_default(),
        }
    }

    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.values.push(value.serialize(Serializer)?);

        Ok(())
    }

    fn finish(self) -> Result<Value> {
        let values = Value::Array(self.values);

        Ok(match self.tag {
            Some(variant) => tagged(variant, values),
            None => values,
        })
    }
}

impl ser::SerializeSeq for Sequence {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value> {
        self.finish()
    }
}

impl ser::SerializeTuple for Sequence {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value> {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Sequence {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value> {
        self.finish()
    }
}

impl ser::SerializeTupleVariant for Sequence {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<Value> {
        self.finish()
    }
}

/// A table being built. The key arrives before its value, so it is held
/// until the value that belongs to it turns up.
struct Table {
    tag: Option<&'static str>,
    entries: BTreeMap<String, Value>,
    key: Option<String>,
}

impl Table {
    fn new(tag: Option<&'static str>) -> Self {
        Self {
            tag,
            entries: BTreeMap::new(),
            key: None,
        }
    }

    fn finish(self) -> Result<Value> {
        let table = Value::Table(self.entries);

        Ok(match self.tag {
            Some(variant) => tagged(variant, table),
            None => table,
        })
    }
}

impl ser::SerializeMap for Table {
    type Ok = Value;
    type Error = Error;

    /// A key is a name, and a name is a string. Anything else has no
    /// spelling in a configuration document, so it is refused here rather
    /// than written as something a reader cannot ask for again.
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<()> {
        match key.serialize(Serializer)? {
            Value::String(text) => {
                self.key = Some(text);

                Ok(())
            }
            other => Err(Error(format!(
                "a map key must be a string; this one is {}",
                other.kind()
            ))),
        }
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        let key = self
            .key
            .take()
            .expect("a value is only serialized after its key");

        self.entries.insert(key, value.serialize(Serializer)?);

        Ok(())
    }

    fn end(self) -> Result<Value> {
        self.finish()
    }
}

impl ser::SerializeStruct for Table {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.entries
            .insert(key.to_owned(), value.serialize(Serializer)?);

        Ok(())
    }

    fn end(self) -> Result<Value> {
        self.finish()
    }
}

impl ser::SerializeStructVariant for Table {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.entries
            .insert(key.to_owned(), value.serialize(Serializer)?);

        Ok(())
    }

    fn end(self) -> Result<Value> {
        self.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::to_value;
    use serde::Serialize;

    /// The same value, through the implementation this one was ported from
    /// and then across into this crate's tree — which is exactly what every
    /// caller used to do.
    fn original<T: Serialize>(value: &T) -> Result<crate::Value, String> {
        figment::value::Value::serialize(value)
            .map(|value| crate::backend::figment::from_figment(&value))
            .map_err(|error| error.to_string())
    }

    /// Compared on the value, and on *whether* it failed — the wording of a
    /// refusal is this crate's own.
    #[track_caller]
    fn agrees<T: Serialize>(value: &T) {
        let ours = to_value(value).map_err(|_| ());
        let theirs = original(value).map_err(|_| ());

        assert_eq!(ours, theirs, "the two serializations disagree");
    }

    #[test]
    fn the_scalars_write_the_same() {
        agrees(&true);
        agrees(&8080u16);
        agrees(&-1i8);
        agrees(&i128::MIN);
        agrees(&u128::MAX);
        agrees(&1.5f32);
        agrees(&1.5f64);
        agrees(&'x');
        agrees(&"a string");
        agrees(&());
        agrees(&Option::<u16>::None);
        agrees(&Some(8080u16));
    }

    #[test]
    fn the_shapes_write_the_same() {
        #[derive(Serialize)]
        struct Nested {
            host: String,
            port: u16,
            tags: Vec<String>,
        }

        #[derive(Serialize)]
        #[serde(rename_all = "lowercase")]
        enum Mode {
            Fast,
            Timeout(u16),
            Range(u16, u16),
            Window { from: u16, to: u16 },
        }

        agrees(&Nested {
            host: "0.0.0.0".to_owned(),
            port: 8080,
            tags: vec!["a".to_owned(), "b".to_owned()],
        });
        agrees(&Mode::Fast);
        agrees(&Mode::Timeout(30));
        agrees(&Mode::Range(1, 2));
        agrees(&Mode::Window { from: 1, to: 2 });
        agrees(&(1u8, "two", 3.0f64));
        agrees(&std::collections::BTreeMap::from([("key", 10u8)]));
        agrees(&vec![1u8, 2, 3]);
    }

    /// The shape neither of them can write, refused by both.
    #[test]
    fn a_key_that_is_not_a_name_is_refused_by_both() {
        let map = std::collections::BTreeMap::from([(10u8, "value")]);

        assert!(to_value(&map).is_err());
        assert!(original(&map).is_err());
    }

    /// And the refusal says what kind of key it was, not what was in it.
    #[test]
    fn the_refusal_names_a_kind_and_not_a_value() {
        let map = std::collections::BTreeMap::from([(7331u16, "value")]);
        let error = to_value(&map).expect_err("a number is not a name");

        assert!(!error.to_string().contains("7331"), "{error}");
        assert!(error.to_string().contains("a number"), "{error}");
    }
}

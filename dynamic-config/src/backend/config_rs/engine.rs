//! The `config` crate's fold, behind this crate's `Engine` trait.
//!
//! The backend really does the merging: each layer is added as one of its
//! sources and its own `ConfigBuilder` folds them. What this file is, is
//! the two conversions either side of that, and the one place where the
//! backend's idea of a key had to be kept out of a document's.

use crate::engine::{Engine, Folded, Layer};
use crate::error::Error;
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct ConfigRs;

impl Engine for ConfigRs {
    fn name(&self) -> &str {
        "config-rs"
    }

    fn fold(&self, layers: &[Layer<'_>]) -> Result<Folded, Error> {
        let names = Names::of(layers);
        let mut builder = config_rs::Config::builder();

        for layer in layers {
            builder = builder.add_source(Contribution {
                tag: layer.tag,
                values: names.encode(layer.values),
            });
        }

        let merged = builder
            .build()
            .and_then(|config| config_rs::Source::collect(&config))
            .map_err(|error| super::error::unfolded(&error))?;

        let mut values = BTreeMap::new();
        let mut tags = BTreeMap::new();

        for (token, value) in merged {
            let key = names.decode(&token);
            let mut path = vec![key.clone()];

            values.insert(key, unpack(&value, &mut path, &mut tags));
        }

        Ok(Folded {
            values: Value::Table(values),
            tags,
        })
    }
}

/// A section's own key names, swapped for plain ones while the backend
/// holds them.
///
/// This backend reads a top-level key as a *path expression*, so a
/// section holding `{"my.module": "debug"}` — an ordinary shape, and how
/// half the logging configuration in the world is written — would come
/// back as `{"my": {"module": "debug"}}` here and as itself everywhere
/// else. Standing in a name with nothing special in it keeps the merge
/// the backend's and the keys the document's.
///
/// Only the top level needs this: below it the backend already treats a
/// key as a name rather than a path.
struct Names(Vec<String>);

impl Names {
    /// Every top-level key any layer supplies, in one stable order — so
    /// two layers naming the same key get the same stand-in and still
    /// meet in the merge.
    fn of(layers: &[Layer<'_>]) -> Self {
        let mut names: Vec<String> = layers
            .iter()
            .filter_map(|layer| match layer.values {
                Value::Table(table) => Some(table.keys().cloned()),
                _ => None,
            })
            .flatten()
            .collect();

        names.sort_unstable();
        names.dedup();

        Self(names)
    }

    /// One layer, under names the backend will not read as paths.
    ///
    /// Renamed here and converted at `collect` time rather than both at
    /// once: converting early means the *packed* form is what gets cloned
    /// for the backend, and a packed value carries an origin string per
    /// entry. Measured, it costs 57 more allocations per resolve than
    /// this does.
    fn encode(&self, values: &Value) -> BTreeMap<String, Value> {
        let Value::Table(table) = values else {
            return BTreeMap::new();
        };

        table
            .iter()
            .map(|(key, value)| {
                let token = self
                    .0
                    .binary_search(key)
                    .map_or_else(|_| key.clone(), |index| format!("key{index}"));

                (token, value.clone())
            })
            .collect()
    }

    fn decode(&self, token: &str) -> String {
        token
            .strip_prefix("key")
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| self.0.get(index).cloned())
            .unwrap_or_else(|| token.to_owned())
    }
}

/// One layer, as a source the backend can read.
///
/// The tag rides along as each value's origin — the backend keeps it on
/// whatever survives the merge, which is exactly the question being
/// asked.
#[derive(Debug, Clone)]
struct Contribution {
    tag: usize,
    values: BTreeMap<String, Value>,
}

impl config_rs::Source for Contribution {
    fn clone_into_box(&self) -> Box<dyn config_rs::Source + Send + Sync> {
        Box::new(self.clone())
    }

    fn collect(&self) -> Result<config_rs::Map<String, config_rs::Value>, config_rs::ConfigError> {
        let tag = self.tag.to_string();

        Ok(self
            .values
            .iter()
            .map(|(key, value)| (key.clone(), pack(value, &tag)))
            .collect())
    }
}

/// This crate's tree, as the backend's — tagged where a tag is read.
///
/// The origin is a `String` on every value the backend holds, and this
/// map is cloned once more when the backend asks for it — so tagging a
/// table whose tag nobody reads is two allocations per key, per fold,
/// for nothing. `unpack` reads a tag at a leaf and at an *empty* table
/// (which is a leaf here), and descends through every other table
/// without looking.
<<<<<<< HEAD
fn pack(value: &Value, tag: &String) -> config_rs::Value {
=======
fn pack(value: &Value, tag: &str) -> config_rs::Value {
>>>>>>> origin/main
    let tagged = !matches!(value, Value::Table(table) if !table.is_empty());
    let kind = match value {
        Value::Null => config_rs::ValueKind::Nil,
        Value::Bool(boolean) => config_rs::ValueKind::Boolean(*boolean),
        // Narrowed rather than always `I128`: a backend that has one
        // integer per width should be handed the narrowest that fits,
        // so a round trip comes back the width it went in as.
        Value::Integer(number) => i64::try_from(*number).map_or_else(
            |_| {
                u64::try_from(*number).map_or(
                    config_rs::ValueKind::I128(*number),
                    config_rs::ValueKind::U64,
                )
            },
            config_rs::ValueKind::I64,
        ),
        Value::Float(number) => config_rs::ValueKind::Float(*number),
        Value::String(text) => config_rs::ValueKind::String(text.clone()),
        Value::Array(values) => {
            config_rs::ValueKind::Array(values.iter().map(|value| pack(value, tag)).collect())
        }
        Value::Table(table) => config_rs::ValueKind::Table(
            table
                .iter()
                .map(|(key, value)| (key.clone(), pack(value, tag)))
                .collect(),
        ),
    };

    if tagged {
<<<<<<< HEAD
        // `&String` all the way down rather than `&str` plus a
        // `to_owned` here: the backend clones the origin into every value it
        // builds, so allocating one to hand it was a second allocation per
        // leaf, per layer, per fold.
        config_rs::Value::new(Some(tag), kind)
=======
        config_rs::Value::new(Some(&tag.to_owned()), kind)
>>>>>>> origin/main
    } else {
        config_rs::Value::new(None, kind)
    }
}

/// The backend's tree, as this crate's — recording each leaf's tag on
/// the way past it.
fn unpack(
    value: &config_rs::Value,
    path: &mut Vec<String>,
    tags: &mut BTreeMap<String, usize>,
) -> Value {
    if let config_rs::ValueKind::Table(table) = &value.kind {
        // A table with keys is a step on the way to a leaf. An *empty*
        // one is the exception and is a leaf itself: a path a layer
        // supplied, holding nothing.
        if table.is_empty() {
            remember(value.origin(), path, tags);

            return Value::Table(BTreeMap::new());
        }

        let mut values = BTreeMap::new();

        for (key, value) in table {
            path.push(key.clone());
            values.insert(key.clone(), unpack(value, path, tags));
            path.pop();
        }

        return Value::Table(values);
    }

    remember(value.origin(), path, tags);

    match &value.kind {
        config_rs::ValueKind::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| unpack(value, path, tags))
                .collect(),
        ),
        config_rs::ValueKind::Table(_) => unreachable!("a table was handled above"),
        _ => super::from_config_rs(value),
    }
}

/// Writes down which layer won this leaf, when the backend still knows.
fn remember(origin: Option<&str>, path: &[String], tags: &mut BTreeMap<String, usize>) {
    if let Some(tag) = origin.and_then(|origin| origin.parse().ok()) {
        tags.insert(path.join("."), tag);
    }
}

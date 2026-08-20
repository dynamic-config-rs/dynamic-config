//! figment's fold, behind this crate's `Engine` trait.
//!
//! figment records metadata per *provider*, so each layer is merged as a
//! provider named after its tag and the winner of a leaf is read back from
//! whichever metadata survived the merge.

use crate::engine::{Engine, Folded, Layer};
use crate::error::{Error, ErrorKind};
use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct Figment;

impl Engine for Figment {
    fn name(&self) -> &str {
        "figment"
    }

    fn fold(&self, layers: &[Layer<'_>]) -> Result<Folded, Error> {
        let mut figment = figment::Figment::new();

        for layer in layers {
            let dict: figment::value::Dict = match layer.values {
                Value::Table(table) => table
                    .iter()
                    .map(|(key, value)| (key.clone(), super::to_figment(value)))
                    .collect(),
                _ => figment::value::Dict::new(),
            };

            // Named by the tag, because metadata is per provider here:
            // the name is how a surviving leaf says which layer it came
            // from, so it has to be the tag and nothing prettier.
            figment = figment.merge(Tagged {
                tag: layer.tag,
                dict,
            });
        }

        let merged: figment::value::Dict = figment.extract().map_err(|_| {
            Error::new(
                ErrorKind::Backend,
                "the resolution engine refused the layers it was given",
            )
        })?;

        let mut values = BTreeMap::new();
        let mut tags = BTreeMap::new();

        for (key, value) in &merged {
            let mut path = vec![key.clone()];

            values.insert(key.clone(), unpack(&figment, value, &mut path, &mut tags));
        }

        Ok(Folded {
            values: Value::Table(values),
            tags,
        })
    }
}

/// One layer, as a provider named after its tag.
struct Tagged {
    tag: usize,
    dict: figment::value::Dict,
}

impl figment::Provider for Tagged {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named(self.tag.to_string())
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, figment::value::Dict>> {
        Ok(figment::value::Map::from([(
            figment::Profile::Default,
            self.dict.clone(),
        )]))
    }
}

/// The backend's tree, as this crate's, asking it which provider won
/// each leaf as the walk passes one.
fn unpack(
    figment: &figment::Figment,
    value: &figment::value::Value,
    path: &mut Vec<String>,
    tags: &mut BTreeMap<String, usize>,
) -> Value {
    if let figment::value::Value::Dict(_, dict) = value {
        // As above: an empty table is a leaf, and the layer that put it
        // there is the answer for that path.
        if dict.is_empty() {
            remember(figment, path, tags);

            return Value::Table(BTreeMap::new());
        }

        let mut values = BTreeMap::new();

        for (key, value) in dict {
            path.push(key.clone());
            values.insert(key.clone(), unpack(figment, value, path, tags));
            path.pop();
        }

        return Value::Table(values);
    }

    remember(figment, path, tags);

    super::from_figment(value)
}

/// Which provider's value survived at this path.
fn remember(figment: &figment::Figment, path: &[String], tags: &mut BTreeMap<String, usize>) {
    let dotted = path.join(".");

    if let Some(tag) = figment
        .find_metadata(&dotted)
        .and_then(|metadata| metadata.name.parse().ok())
    {
        tags.insert(dotted, tag);
    }
}

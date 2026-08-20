//! A figment provider read as one layer of a load.
//!
//! The other half of `Source::provider`, which is the one door figment
//! appears at in this crate's own API. A provider hands over values filed
//! by *profile*, and this crate files them by section — so the three
//! profiles that can mean "this section" are read and merged in the order
//! figment merges them.

use std::collections::BTreeMap;

use crate::error::Error;
use crate::value::Value;

/// A foreign provider's document, in this crate's tree.
fn into_tree(dict: &figment::value::Dict) -> BTreeMap<String, Value> {
    dict.iter()
        .map(|(key, value)| (key.clone(), super::from_figment(value)))
        .collect()
}

/// A foreign provider's section: the values it files under this section's
/// name, over the ones it files under no name at all.
///
/// A provider written against the backend names its sections the way that
/// backend names profiles, so all three are read and merged in the order
/// that backend merges them: the unnamed default first, this section's own
/// name over it, and a global say over both.
pub(crate) fn section_of(
    provider: &(dyn figment::Provider + Send + Sync),
    key: &str,
) -> Result<Option<BTreeMap<String, Value>>, Error> {
    let data = provider
        .data()
        .map_err(|error| super::error::translate(&error))?;

    let mut found: Option<crate::Value> = None;

    for candidate in [
        figment::Profile::Default,
        figment::Profile::from(key),
        figment::Profile::from("global"),
    ] {
        let Some(dict) = data.get(&candidate) else {
            continue;
        };

        let values = crate::Value::Table(into_tree(dict));

        match &mut found {
            Some(into) => into.merge(values),
            None => found = Some(values),
        }
    }

    match found {
        Some(crate::Value::Table(table)) => Ok(Some(table)),
        _ => Ok(None),
    }
}

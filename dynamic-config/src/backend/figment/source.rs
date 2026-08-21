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

/// Where a foreign provider says its values come from.
///
/// **The metadata's *source*, not its name** — which is what
/// [`Source::provider`](crate::Source::provider) documents, and what was
/// being thrown away: every provider answered `Inline`, however carefully
/// it described itself. A name reaches error messages; a source reaches
/// `source_of` and `explain`, and those are what tell somebody which file
/// to go and edit.
pub(crate) fn origin_of(provider: &(dyn figment::Provider + Send + Sync)) -> crate::Origin {
    match provider.metadata().source {
        // The documented case: `Metadata::from("INI file", path)`, and the
        // value traces back to the file exactly as one from `.file(..)`.
        Some(figment::Source::File(path)) => crate::Origin::File(path),
        // Values written in code are what `Inline` already means here.
        Some(figment::Source::Code(_)) => crate::Origin::Inline,
        // A provider that describes its source in its own words: a store, a
        // socket, a database. `Remote` is the variant that renders as
        // "from {what}", which is what such a description is.
        Some(figment::Source::Custom(what)) => crate::Origin::Remote(what),
        // Described by name alone, or not at all. Honest beats invented —
        // and the same answer covers a source kind this backend adds later:
        // the enum is `#[non_exhaustive]`, so a new variant is a thing this
        // crate has never seen rather than a thing it may guess at.
        Some(_) | None => crate::Origin::Unknown,
    }
}

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

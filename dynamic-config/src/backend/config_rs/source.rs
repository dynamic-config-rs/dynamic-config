//! A `config` source read as one layer of a load.
//!
//! The other half of [`Source::config_source`](crate::Source::config_source),
//! which is the one door this backend appears at in the crate's own API —
//! the twin of figment's, and there for the same reason: a store, a
//! format or a shape this crate does not ship, wired in without forking
//! it.
//!
//! **Simpler than figment's twin, because the backend is simpler here.**
//! figment files values under *profiles*, so reading one means deciding
//! which profiles mean "this section". A `config` source hands over a
//! flat map of keys and nothing else, so what it collects is what the
//! layer is.

use std::collections::BTreeMap;

use crate::error::Error;
use crate::value::Value;

/// Everything the source has to say, as this crate's tree.
///
/// # Errors
///
/// If the source refuses to collect — a file it cannot read, a document
/// it cannot parse.
pub(crate) fn layer(
    source: &(dyn config_rs::Source + Send + Sync),
) -> Result<BTreeMap<String, Value>, Error> {
    let collected = source
        .collect()
        .map_err(|error| super::error::unparsed(&error))?;

    Ok(collected
        .into_iter()
        .map(|(key, value)| (key, super::from_config_rs(&value)))
        .collect())
}

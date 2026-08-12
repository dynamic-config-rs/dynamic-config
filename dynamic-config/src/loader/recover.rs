//! Recovery: loading with the cache standing in for the files that broke.

use figment::value::Dict;
use figment::Figment;
use serde::de::DeserializeOwned;

use crate::error::Error;
use crate::layer::{FLAGS_NAME, OVERRIDES_NAME};
use crate::snapshot::Snapshot;
use crate::source::LoadSpec;

use super::aliases_pass::apply_aliases;
use super::environment::{environment, merge_env_files};
use super::origin::convert;
use super::sections::Cached;

/// Loads with `cached` standing in for the files.
///
/// The files are what broke, so recovery reads none of them — a malformed file
/// fails to parse whatever sits underneath it. The environment and the runtime
/// layers are applied over the cache exactly as they would be over the files,
/// which is what lets a redacted cache work: the values it dropped come back
/// from wherever they were live.
pub(crate) fn recover<T: DeserializeOwned>(
    spec: &LoadSpec<'_>,
    cached: &Snapshot,
) -> Result<(T, Snapshot), Error> {
    let mut figment = Figment::new().merge(Cached {
        values: cached.values().clone(),
        profile: figment::Profile::from(super::section_profile(spec.key)),
    });

    // The same order as `build()`, deliberately: `.env` files first, the real
    // environment after, so a variable somebody exported to steer the
    // recovery beats a file in the repository — recovery is exactly the
    // moment a human is overriding things by hand.
    figment = merge_env_files(figment, spec)?;

    if let Some(prefix) = spec.full_env_prefix() {
        // The invariant a caller opted into does not get suspended on the
        // fallback path: an ambiguous spelling refused during the normal
        // load is refused during recovery too.
        if spec.strict_env {
            super::environment::reject_ambiguous(&prefix)?;
        }

        figment = figment.merge(environment(
            &prefix,
            spec.key,
            spec.nest,
            spec.allow_empty_env,
        ));
    }

    if let Some(bindings) = spec.env_bindings {
        let fallback = super::env_file_entries(spec)?;

        for binding in bindings.providers(spec.key, spec.allow_empty_env, fallback) {
            figment = figment.merge(binding);
        }
    }

    if let Some(flags) = spec.flags {
        figment = figment.merge(flags.provider(spec.key, FLAGS_NAME));
    }

    if let Some(overrides) = spec.overrides {
        figment = figment.merge(overrides.provider(spec.key, OVERRIDES_NAME));
    }

    let figment = apply_aliases(figment, spec).select(super::section_profile(spec.key));

    // The resolved tree rides along so the caller can seed the diff baseline:
    // without it, the first successful reload after a recovery had nothing to
    // compare against and reported no changes — untrue, and at the worst time.
    let resolved: Dict = figment.extract().map_err(|error| convert(error, spec))?;
    let config: T = figment.extract().map_err(|error| convert(error, spec))?;

    Ok((config, Snapshot::new(resolved)))
}

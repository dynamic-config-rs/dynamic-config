//! The loader, built on [`figment`](https://docs.rs/figment).
//!
//! figment is the only backend. It already solves layered providers, profile
//! selection and loose typing of environment values; reimplementing that would
//! mean maintaining a second set of edge cases that behave *almost* the same.
//!
//! What this module owns is the arrangement:
//!
//! ```text
//! files (left → right, later wins)   →  nested providers
//!                                    →  Env::prefixed(..).split("__")
//!                                    →  select(key)
//!                                    →  extract()
//! ```
//!
//! and the translation of `figment::Error` into this crate's [`Error`], so no
//! figment type reaches a caller's signature.

mod aliases_pass;
mod environment;
mod origin;
mod recover;
mod sections;

use figment::value::Dict;
use figment::Figment;
use std::path::Path;

use serde::de::DeserializeOwned;

use crate::error::{Error, Origin};
use crate::layer::{DEFAULTS_NAME, FLAGS_NAME, OVERRIDES_NAME};
use crate::snapshot::Snapshot;
use crate::source::LoadSpec;

use aliases_pass::apply_aliases;
use environment::{environment, merge_env_files};
use origin::convert;

pub(crate) use recover::recover;

/// Metadata name for the recovery provider.
pub(crate) const CACHED_NAME: &str = "the last configuration that worked";

/// Prefixed onto a remote store's own description, so that a value traced back
/// to one names the store rather than reporting "an inline source" — which is
/// what figment sees, and is the wrong answer to the question being asked.
const REMOTE_PREFIX: &str = "the remote store ";

pub(crate) fn load<T: DeserializeOwned>(spec: &LoadSpec<'_>) -> Result<T, Error> {
    apply_aliases(build(spec)?, spec)
        .select(spec.key)
        .extract()
        .map_err(convert)
}

/// Resolves the section without deserializing it.
pub(crate) fn snapshot(spec: &LoadSpec<'_>) -> Result<Snapshot, Error> {
    resolved(spec).map(|(snapshot, _figment)| snapshot)
}

/// The resolved section together with the figment it came from.
///
/// For a caller that wants to ask [`origin_in`] many times: `check()` reports
/// the origin of every leaf key, and building the figment per question would
/// re-read and re-parse every source once per key — O(keys × sources) file
/// I/O for one report. `extract` takes `&self`, so the figment survives it.
pub(crate) fn resolved(spec: &LoadSpec<'_>) -> Result<(Snapshot, Figment), Error> {
    let figment = apply_aliases(build(spec)?, spec).select(spec.key);
    let snapshot = figment
        .extract::<Dict>()
        .map(Snapshot::new)
        .map_err(convert)?;

    Ok((snapshot, figment))
}

/// Where `path` comes from, in a figment [`resolved`] already built.
pub(crate) fn origin_in(figment: &Figment, path: &str) -> Origin {
    figment
        .find_metadata(path)
        .map_or(Origin::Unknown, origin::origin_of)
}

/// Where the value at `path` would come from, if anywhere.
pub(crate) fn source_of(spec: &LoadSpec<'_>, path: &str) -> Result<Option<Origin>, Error> {
    let figment = apply_aliases(build(spec)?, spec).select(spec.key);

    Ok(figment.find_metadata(path).map(origin::origin_of))
}

/// Whether anything supplies `path`.
pub(crate) fn is_set(spec: &LoadSpec<'_>, path: &str) -> Result<bool, Error> {
    Ok(apply_aliases(build(spec)?, spec)
        .select(spec.key)
        .contains(path))
}

/// Assembles the providers for `spec`, in precedence order.
fn build(spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let mut figment = Figment::new();

    // Merged first, so anything at all displaces them.
    if let Some(defaults) = spec.defaults {
        figment = figment.merge(defaults.provider(spec.key, DEFAULTS_NAME));
    }

    let profile = sections::validated_profile(spec)?;

    // Discovered files sit below the explicitly listed ones: `files = [..]` is
    // a deliberate statement, a search result is a guess about the machine.
    if let Some(search) = &spec.search {
        for (path, format) in search.resolve() {
            figment = sections::merge_file(figment, &path, format)?;
            figment = sections::merge_profile_variant(figment, &path, format, profile.as_deref())?;
        }
    }

    for source in spec.sources {
        figment = sections::merge(figment, source)?;

        if let (Some(path), Some(format)) = (source.path(), source.format()) {
            figment = sections::merge_profile_variant(
                figment,
                Path::new(path),
                format,
                profile.as_deref(),
            )?;
        }
    }

    // Above the files: what a central store distributes should beat what a
    // package shipped. Below the environment, which comes next.
    if let Some(remote) = spec.remote {
        if let Some(document) = remote.document() {
            let name = format!(
                "{REMOTE_PREFIX}{}",
                remote.describe().unwrap_or_else(|| "(unnamed)".to_owned())
            );

            figment =
                sections::merge_named_text(figment, &document.text, document.format, &name, None)?;
        }
    }

    // A `.env` is the environment layer sourced from disk, so it goes just
    // below the real thing: a variable somebody exported for this run beats a
    // file in the repository.
    figment = merge_env_files(figment, spec)?;

    // Filed under the same profile the files use, so one `select` sees both.
    // Merged after every file, so the environment wins over all of them.
    if let Some(prefix) = spec.full_env_prefix() {
        figment = figment.merge(environment(
            &prefix,
            spec.key,
            spec.nest,
            spec.allow_empty_env,
        ));
    }

    // Above the environment: a flag is typed by a person for this one run, and
    // should win over whatever the deployment happens to export.

    if let Some(bindings) = spec.env_bindings {
        for binding in bindings.providers(spec.key, spec.allow_empty_env) {
            figment = figment.merge(binding);
        }
    }

    if let Some(flags) = spec.flags {
        figment = figment.merge(flags.provider(spec.key, FLAGS_NAME));
    }

    // Merged last, so nothing displaces them. This is what makes a test
    // authoritative without editing anything on disk.
    if let Some(overrides) = spec.overrides {
        figment = figment.merge(overrides.provider(spec.key, OVERRIDES_NAME));
    }

    Ok(figment)
}

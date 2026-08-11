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
    merged(spec)?.extract().map_err(convert)
}

/// The composed figment: every layer merged, aliases applied, the section
/// selected. What every question below starts from.
pub(crate) fn merged(spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    Ok(apply_aliases(build(spec)?, spec).select(spec.key))
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
    let figment = merged(spec)?;
    let mut snapshot = figment
        .extract::<Dict>()
        .map(Snapshot::new)
        .map_err(convert)?;

    // The one moment the figment that knows where every value came from is
    // still alive — asked for every leaf before it is dropped, so the
    // snapshot can answer `source_of` for itself later.
    let provenance = snapshot
        .leaf_paths()
        .into_iter()
        .filter_map(|path| match origin_in(&figment, &path) {
            // An `Unknown` row answers the question with a shrug; absence
            // says the same without taking up a slot.
            Origin::Unknown => None,
            origin => Some((path, origin)),
        })
        .collect();
    snapshot.attach_provenance(provenance);

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
    Ok(merged(spec)?.find_metadata(path).map(origin::origin_of))
}

/// Whether anything supplies `path`.
pub(crate) fn is_set(spec: &LoadSpec<'_>, path: &str) -> Result<bool, Error> {
    Ok(merged(spec)?.contains(path))
}

/// One layer of the precedence order.
///
/// A layer knows three things: what it is called in diagnostics, whether the
/// spec configures it at all, and how it merges itself into a figment. Both
/// [`build`] and [`explain`](crate::explain) walk the same table, so the
/// composed load and the per-layer explanation cannot disagree about what the
/// layers are or which order they come in.
pub(crate) struct LayerDef {
    name: &'static str,
    active: fn(&LoadSpec<'_>) -> bool,
    merge: fn(Figment, &LoadSpec<'_>) -> Result<Figment, Error>,
}

/// **The precedence order lives here and nowhere else.** Adding a layer means
/// adding a row, and the position needs an argument in a comment.
const LAYERS: &[LayerDef] = &[
    // Merged first, so anything at all displaces them.
    LayerDef {
        name: "default",
        active: |spec| spec.defaults.is_some_and(|layer| !layer.is_empty()),
        merge: merge_defaults,
    },
    // Discovered files sit below the explicitly listed ones: `files = [..]`
    // is a deliberate statement, a search result is a guess about the
    // machine.
    LayerDef {
        name: "discovered",
        active: |spec| spec.search.is_some(),
        merge: merge_discovered,
    },
    LayerDef {
        name: "file",
        active: |spec| !spec.sources.is_empty(),
        merge: merge_listed,
    },
    // Above the files: what a central store distributes should beat what a
    // package shipped. Below the environment, which comes next.
    LayerDef {
        name: "remote",
        active: |spec| {
            spec.remote
                .is_some_and(|remote| remote.document().is_some())
        },
        merge: merge_remote,
    },
    // A `.env` is the environment layer sourced from disk, so it goes just
    // below the real thing: a variable somebody exported for this run beats
    // a file in the repository.
    LayerDef {
        name: ".env",
        active: |spec| !spec.env_files.is_empty(),
        merge: merge_env_files,
    },
    // Filed under the same profile the files use, so one `select` sees both.
    // Merged after every file, so the environment wins over all of them.
    LayerDef {
        name: "environment",
        active: |spec| spec.full_env_prefix().is_some(),
        merge: merge_environment,
    },
    // Above the environment: a binding, like a flag, is a deliberate act of
    // wiring rather than whatever the deployment happens to export.
    LayerDef {
        name: "binding",
        active: |spec| {
            spec.env_bindings
                .is_some_and(|bindings| !bindings.is_empty())
        },
        merge: merge_bindings,
    },
    // A flag is typed by a person for this one run.
    LayerDef {
        name: "flag",
        active: |spec| spec.flags.is_some_and(|layer| !layer.is_empty()),
        merge: merge_flags,
    },
    // Merged last, so nothing displaces them. This is what makes a test
    // authoritative without editing anything on disk.
    LayerDef {
        name: "override",
        active: |spec| spec.overrides.is_some_and(|layer| !layer.is_empty()),
        merge: merge_overrides,
    },
];

/// Assembles the providers for `spec`, in precedence order.
fn build(spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let mut figment = Figment::new();

    for layer in LAYERS {
        if (layer.active)(spec) {
            figment = (layer.merge)(figment, spec)?;
        }
    }

    Ok(figment)
}

/// Every active layer's name next to a figment holding only that layer —
/// what [`crate::explain`] walks to answer "who had what to say".
///
/// "Active" means the layer has something to say at all: the generated code
/// wires every runtime layer unconditionally, so `Some` alone would put an
/// empty `flag` row in every table. Content decides, not wiring.
pub(crate) fn layer_figments(spec: &LoadSpec<'_>) -> Result<Vec<(&'static str, Figment)>, Error> {
    LAYERS
        .iter()
        .filter(|layer| (layer.active)(spec))
        .map(|layer| {
            Ok((
                layer.name,
                (layer.merge)(Figment::new(), spec)?.select(spec.key),
            ))
        })
        .collect()
}

fn merge_defaults(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    match spec.defaults {
        Some(defaults) => Ok(figment.merge(defaults.provider(spec.key, DEFAULTS_NAME))),
        None => Ok(figment),
    }
}

fn merge_discovered(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let profile = sections::validated_profile(spec)?;

    if let Some(search) = &spec.search {
        for (path, format) in search.resolve() {
            figment = sections::merge_file(figment, &path, format)?;
            figment = sections::merge_profile_variant(figment, &path, format, profile.as_deref())?;
        }
    }

    Ok(figment)
}

fn merge_listed(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let profile = sections::validated_profile(spec)?;

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

    Ok(figment)
}

fn merge_remote(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    if let Some(remote) = spec.remote {
        if let Some(document) = remote.document() {
            let name = format!(
                "{REMOTE_PREFIX}{}",
                remote.describe().unwrap_or_else(|| "(unnamed)".to_owned())
            );

            return sections::merge_named_text(
                figment,
                &document.text,
                document.format,
                &name,
                None,
            );
        }
    }

    Ok(figment)
}

fn merge_environment(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    if spec.strict_env {
        if let Some(prefix) = spec.full_env_prefix() {
            environment::reject_ambiguous(&prefix)?;
        }
    }

    match spec.full_env_prefix() {
        Some(prefix) => Ok(figment.merge(environment(
            &prefix,
            spec.key,
            spec.nest,
            spec.allow_empty_env,
        ))),
        None => Ok(figment),
    }
}

fn merge_bindings(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    if let Some(bindings) = spec.env_bindings {
        for binding in bindings.providers(spec.key, spec.allow_empty_env) {
            figment = figment.merge(binding);
        }
    }

    Ok(figment)
}

fn merge_flags(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    match spec.flags {
        Some(flags) => Ok(figment.merge(flags.provider(spec.key, FLAGS_NAME))),
        None => Ok(figment),
    }
}

fn merge_overrides(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    match spec.overrides {
        Some(overrides) => Ok(figment.merge(overrides.provider(spec.key, OVERRIDES_NAME))),
        None => Ok(figment),
    }
}

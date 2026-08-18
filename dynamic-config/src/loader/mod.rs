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
// `properties` borrows `ini`'s nested-insert and scalar-widening helpers,
// so the module compiles for either feature; the `Ini` provider itself is
// gated inside.
#[cfg(any(feature = "ini", feature = "properties"))]
mod ini;
mod origin;
#[cfg(feature = "properties")]
mod properties;
mod recover;
mod secrets;
pub(crate) mod sections;

use figment::value::Dict;
use figment::Figment;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::error::{Error, Origin};
use crate::layer::{DEFAULTS_NAME, FLAGS_NAME, OVERRIDES_NAME};
use crate::snapshot::Snapshot;
use crate::source::LoadSpec;

use aliases_pass::apply_aliases;
use environment::{environment, merge_env_files};
use origin::convert;
use secrets::merge_secrets_dir;

pub(crate) use origin::translate;
pub(crate) use recover::recover;
pub(crate) use sections::parse_document;

/// Metadata name for the recovery provider.
pub(crate) const CACHED_NAME: &str = "the last configuration that worked";

/// The figment profile a section's data files under.
///
/// Prefixed, because figment reserves two profile names with inheritance
/// semantics — `global` overrides every profile's own values, `default`
/// seeds them — and an unprefixed mapping handed those powers to any
/// config file with an innocently named top-level table, silently and
/// invisibly to every diagnostic. Both sides go through here: the `Sections`
/// provider files each top-level key this way, and every select asks the
/// same way, so a user's key can never collide with figment's machinery.
pub(crate) fn section_profile(key: &str) -> String {
    format!("section:{key}")
}

/// Prefixed onto a remote store's own description, so that a value traced back
/// to one names the store rather than reporting "an inline source" — which is
/// what figment sees, and is the wrong answer to the question being asked.
const REMOTE_PREFIX: &str = "the remote store ";

pub(crate) fn load<T: DeserializeOwned>(spec: &LoadSpec<'_>) -> Result<T, Error> {
    merged(spec)?
        .extract()
        .map_err(|error| convert(error, spec))
}

/// The composed figment: every layer merged, aliases applied, the section
/// selected. What every question below starts from.
pub(crate) fn merged(spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    Ok(apply_aliases(build(spec)?, spec).select(section_profile(spec.key)))
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
        .map_err(|error| convert(error, spec))?;

    // The one moment the figment that knows where every value came from is
    // still alive — asked for every leaf before it is dropped, so the
    // snapshot can answer `source_of` for itself later.
    let provenance = snapshot
        .leaf_paths()
        .into_iter()
        .filter_map(|path| match origin_in(&figment, &path, spec.nest) {
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
pub(crate) fn origin_in(figment: &Figment, path: &str, nest: &str) -> Origin {
    origin::refine_env(
        figment
            .find_metadata(path)
            .map_or(Origin::Unknown, origin::origin_of),
        path.split('.'),
        nest,
    )
}

/// Where the value at `path` would come from, if anywhere.
pub(crate) fn source_of(spec: &LoadSpec<'_>, path: &str) -> Result<Option<Origin>, Error> {
    Ok(merged(spec)?.find_metadata(path).map(|metadata| {
        origin::refine_env(origin::origin_of(metadata), path.split('.'), spec.nest)
    }))
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
    // Above the remote store and below the environment, which is the same
    // argument made twice: a mounted secret is a fact about *this*
    // deployment, so it beats a document a central store hands to every
    // deployment alike — and loses to a variable exported for this one run,
    // which is more specific still. pydantic-settings agrees on the second
    // half and has no remote layer to disagree about the first.
    LayerDef {
        name: "secrets",
        active: |spec| spec.secrets_dir.is_some(),
        merge: merge_secrets_dir,
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
    // Unconditional, not positional: a path-shaped profile must be rejected
    // whether or not any layer that *uses* it is active — an env-only load
    // with `profile_env` pointing at `../secrets` is exactly the load that
    // must not wait for a file layer to notice.
    sections::validated_profile(spec)?;

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
    // The same unconditional guard as `build` — the two walk the same table
    // and must refuse the same profiles.
    sections::validated_profile(spec)?;

    LAYERS
        .iter()
        .filter(|layer| (layer.active)(spec))
        .map(|layer| {
            Ok((
                layer.name,
                (layer.merge)(Figment::new(), spec)?.select(section_profile(spec.key)),
            ))
        })
        .collect()
}

fn merge_defaults(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    match spec.defaults {
        Some(defaults) => {
            Ok(figment.merge(defaults.provider(&section_profile(spec.key), DEFAULTS_NAME)))
        }
        None => Ok(figment),
    }
}

fn merge_discovered(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let profile = sections::validated_profile(spec)?;
    let layout = sections::Layout::of(spec);

    if let Some(search) = &spec.search {
        for (path, format) in search.resolve() {
            figment = sections::merge_file(figment, &path, format, layout)?;
            figment = sections::merge_profile_variant(
                figment,
                &path,
                format,
                profile.as_deref(),
                layout,
            )?;
        }
    }

    Ok(figment)
}

fn merge_listed(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let profile = sections::validated_profile(spec)?;
    let layout = sections::Layout::of(spec);

    for source in spec.sources {
        figment = sections::merge(figment, source, layout)?;

        if let (Some(path), Some(format)) = (source.path(), source.format()) {
            figment = sections::merge_profile_variant(
                figment,
                Path::new(path),
                format,
                profile.as_deref(),
                layout,
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
                sections::Layout::of(spec),
            );
        }
    }

    Ok(figment)
}

fn merge_environment(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    match spec.full_env_prefix() {
        Some(prefix) => {
            if spec.strict_env {
                environment::reject_ambiguous(&prefix)?;
            }

            Ok(figment.merge(environment(
                &prefix,
                spec.key,
                spec.nest,
                spec.allow_empty_env,
            )))
        }
        None => Ok(figment),
    }
}

fn merge_bindings(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    if let Some(bindings) = spec.env_bindings {
        let fallback = env_file_entries(spec)?;

        for binding in bindings.providers(spec.key, spec.allow_empty_env, fallback) {
            figment = figment.merge(binding);
        }
    }

    Ok(figment)
}

/// What the `.env` files say, for the bindings to fall back to.
///
/// A binding names one variable exactly, and a deployment that writes that
/// variable into a `.env` file rather than exporting it means the same thing
/// by it. The prefixed `.env` layer cannot serve that: it recognises only
/// names built from the prefix and the key, and it is skipped altogether when
/// there is no prefix — which is exactly the shape a program that binds by
/// name tends to have.
///
/// Read here rather than threaded down from the `.env` layer: the two are
/// independent, this one runs whether or not that one did, and a `.env` file
/// is small enough that reading it twice per load costs less than the plumbing
/// that would avoid it.
#[cfg(feature = "dotenv")]
pub(crate) fn env_file_entries(
    spec: &LoadSpec<'_>,
) -> Result<Arc<BTreeMap<String, String>>, Error> {
    let bound = spec
        .env_bindings
        .is_some_and(|bindings| !bindings.is_empty());

    if spec.env_files.is_empty() || !bound {
        return Ok(Arc::default());
    }

    let mut entries = BTreeMap::new();

    // Later files win, as they do when the `.env` layer merges them.
    for file in spec.env_files {
        entries.extend(crate::dotenv::read(Path::new(file))?);
    }

    Ok(Arc::new(entries))
}

/// Without the feature there is nothing to read: `merge_env_files` has already
/// refused any `.env` file the caller configured.
#[cfg(not(feature = "dotenv"))]
pub(crate) fn env_file_entries(
    _spec: &LoadSpec<'_>,
) -> Result<Arc<BTreeMap<String, String>>, Error> {
    Ok(Arc::default())
}

fn merge_flags(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    match spec.flags {
        Some(flags) => Ok(figment.merge(flags.provider(&section_profile(spec.key), FLAGS_NAME))),
        None => Ok(figment),
    }
}

fn merge_overrides(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    match spec.overrides {
        Some(overrides) => {
            Ok(figment.merge(overrides.provider(&section_profile(spec.key), OVERRIDES_NAME)))
        }
        None => Ok(figment),
    }
}

/// Fuzzing doors — see `crate::__fuzz`.
#[cfg(feature = "ini")]
pub(crate) fn __fuzz_ini(text: &str) -> ini::Ini {
    ini::Ini::string(text)
}

#[cfg(feature = "properties")]
pub(crate) fn __fuzz_properties(text: &str) -> properties::Properties {
    properties::Properties::string(text)
}

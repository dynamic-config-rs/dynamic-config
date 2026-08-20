//! The loader: every source's say, folded in precedence order.
//!
//! One walk collects a *contribution* per layer — a tree of what that layer
//! has to say about the section being loaded, and where it said it — and
//! [`resolve::compose`](crate::resolve::compose) folds them lowest to
//! highest, recording the layer that wins each leaf as it wins it. Nothing
//! afterwards has to guess where a value came from:
//!
//! ```text
//! defaults · discovered · files · remote · secrets · .env
//!          · environment · bindings · flags · overrides
//!     │
//!     └── one contribution each ──▶ fold ──▶ (tree, origin per leaf)
//!                                              │
//!                                              └── aliases, then the snapshot
//! ```
//!
//! **The precedence order lives in [`contributions`] and nowhere else.**
//! Adding a layer means adding a call, and the position needs an argument in
//! a comment beside it.

mod aliases_pass;
mod environment;
// `properties` borrows `ini`'s nested-insert and scalar-widening helpers,
// so the module compiles for either feature; the `Ini` provider itself is
// gated inside.
#[cfg(any(feature = "ini", feature = "properties"))]
pub(crate) mod ini;
pub(crate) mod origin;
#[cfg(feature = "properties")]
pub(crate) mod properties;
mod recover;
mod secrets;
pub(crate) mod sections;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::error::{Error, Origin};
use crate::snapshot::Snapshot;
use crate::source::LoadSpec;

use aliases_pass::apply_aliases;

pub(crate) use recover::recover;

pub(crate) fn load<T: DeserializeOwned>(spec: &LoadSpec<'_>) -> Result<T, Error> {
    let snapshot = resolved(spec)?;

    // A snapshot on its own reports the path and not the source, because it
    // has been handed around without them. Here the sources are still in
    // reach, so the leaf that failed is looked up and the error says which
    // file or variable to go and fix.
    snapshot.extract().map_err(|error| {
        let path = error.path();

        match snapshot.source_of(&path) {
            Some(origin) if !path.is_empty() => error.with_origin(origin.clone()),
            _ => error,
        }
    })
}

/// Resolves the section without deserializing it.
pub(crate) fn snapshot(spec: &LoadSpec<'_>) -> Result<Snapshot, Error> {
    resolved(spec)
}

/// Every layer, folded in precedence order, with the record of who won what.
///
/// One walk of the sources answers every question below: `check()` reports
/// the origin of every leaf, and asking a source per key would re-read and
/// re-parse every file once per key — O(keys × sources) of file I/O for one
/// report. The snapshot carries its provenance with it instead.
pub(crate) fn resolved(spec: &LoadSpec<'_>) -> Result<Snapshot, Error> {
    let engine = spec.engine();
    let mut collected = contributions(spec)?;
    let (mut tree, mut provenance) = crate::resolve::compose(engine, collected.take_layers())?;

    apply_aliases(&mut tree, &mut provenance, &collected, spec);

    // The environment layer names the prefix; a leaf it supplied can name the
    // variable, which is the answer somebody is actually looking for.
    for (path, origin) in &mut provenance {
        if let crate::Origin::Env(_) = origin {
            *origin = origin::refine_env(origin.clone(), path.split('.'), spec.nest);
        }
    }

    let mut snapshot = Snapshot::new(tree);
    snapshot.attach_provenance(provenance);

    Ok(snapshot)
}

/// A parser's own account of a failure, with anything it quoted from the
/// document taken out.
///
/// The line a parser stopped on is frequently the line holding the
/// password, and a message is the one place a configuration value has no
/// business appearing.
pub(crate) fn redacted(message: &str) -> String {
    origin::without_backticked_values(message)
}

/// An environment origin narrowed from the prefix to the variable that
/// actually supplied this leaf.
pub(crate) fn refine(origin: Origin, path: &str, nest: &str) -> Origin {
    origin::refine_env(origin, path.split('.'), nest)
}

pub(crate) fn source_of(spec: &LoadSpec<'_>, path: &str) -> Result<Option<Origin>, Error> {
    Ok(resolved(spec)?.source_of(path).cloned())
}

/// Whether anything supplies `path`.
pub(crate) fn is_set(spec: &LoadSpec<'_>, path: &str) -> Result<bool, Error> {
    Ok(resolved(spec)?.contains(path))
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

/// Fuzzing doors — see `crate::__fuzz`.
#[cfg(feature = "ini")]
pub(crate) fn __fuzz_ini(text: &str) -> Result<crate::Value, Error> {
    ini::parse(text)
}

#[cfg(feature = "properties")]
pub(crate) fn __fuzz_properties(text: &str) -> Result<crate::Value, Error> {
    properties::parse(text)
}

// ---------------------------------------------------------------------------
// Contributions: every layer's say, in this crate's own tree.
//
// The composition the loader is moving onto. Each function here answers the
// same question the matching `merge_*` above answers, in the shape
// `resolve::compose` folds — and `tests/composition.rs` composes a spec both
// ways and compares the tree and the winner of every leaf.
// ---------------------------------------------------------------------------

/// Every layer's contribution, in precedence order.
pub(crate) fn contributions(spec: &LoadSpec<'_>) -> Result<crate::resolve::Collected, Error> {
    // Unconditional, not positional: a path-shaped profile must be rejected
    // whether or not any layer that *uses* it is active — an env-only load
    // with `profile_env` pointing at `../secrets` is exactly the load that
    // must not wait for a file layer to notice.
    sections::validated_profile(spec)?;

    let mut collected = crate::resolve::Collected::default();

    // Collected first, so anything at all displaces them.
    collect_defaults(&mut collected, spec);
    // Discovered files sit below the explicitly listed ones: `files = [..]`
    // is a deliberate statement, a search result is a guess about the
    // machine.
    collect_discovered(&mut collected, spec)?;
    collect_listed(&mut collected, spec)?;
    // Above the files: what a central store distributes should beat what a
    // package shipped. Below the environment, which comes further down.
    collect_remote(&mut collected, spec)?;
    // Above the remote store and below the environment, which is the same
    // argument made twice: a mounted secret is a fact about *this*
    // deployment, so it beats a document a central store hands to every
    // deployment alike — and loses to a variable exported for this one run,
    // which is more specific still. pydantic-settings agrees on the second
    // half and has no remote layer to disagree about the first.
    secrets::collect(&mut collected, spec)?;
    // A `.env` is the environment layer sourced from disk, so it goes just
    // below the real thing: a variable somebody exported for this run beats
    // a file in the repository.
    environment::collect_env_files(&mut collected, spec)?;
    collect_environment(&mut collected, spec)?;
    // Above the environment: a binding, like a flag, is a deliberate act of
    // wiring rather than whatever the deployment happens to export.
    collect_bindings(&mut collected, spec)?;
    // Flags, then overrides, last of all — a flag is typed by a person for
    // this one run, and nothing displaces an override. That is what makes a
    // test authoritative without editing anything on disk.
    collect_runtime(&mut collected, spec);

    Ok(collected)
}

fn collect_defaults(into: &mut crate::resolve::Collected, spec: &LoadSpec<'_>) {
    if let Some(defaults) = spec.defaults {
        if !defaults.is_empty() {
            into.layer("default", Origin::Runtime("default"), defaults.tree());
        }
    }
}

fn collect_discovered(
    into: &mut crate::resolve::Collected,
    spec: &LoadSpec<'_>,
) -> Result<(), Error> {
    let profile = sections::validated_profile(spec)?;
    let layout = sections::Layout::of(spec);

    if let Some(search) = &spec.search {
        for (path, format) in search.resolve() {
            sections::collect_file(into, "discovered", &path, format, layout, spec.key)?;
            sections::collect_profile_variant(
                into,
                "discovered",
                &path,
                format,
                profile.as_deref(),
                layout,
                spec.key,
            )?;
        }
    }

    Ok(())
}

fn collect_listed(into: &mut crate::resolve::Collected, spec: &LoadSpec<'_>) -> Result<(), Error> {
    let profile = sections::validated_profile(spec)?;
    let layout = sections::Layout::of(spec);

    for source in spec.sources {
        sections::collect_source(into, "file", source, layout, spec.key)?;

        if let (Some(path), Some(format)) = (source.path(), source.format()) {
            sections::collect_profile_variant(
                into,
                "file",
                Path::new(path),
                format,
                profile.as_deref(),
                layout,
                spec.key,
            )?;
        }
    }

    Ok(())
}

fn collect_remote(into: &mut crate::resolve::Collected, spec: &LoadSpec<'_>) -> Result<(), Error> {
    let Some(remote) = spec.remote else {
        return Ok(());
    };

    let Some(document) = remote.document() else {
        return Ok(());
    };

    let store = remote.describe().unwrap_or_else(|| "(unnamed)".to_owned());
    let parsed = crate::document::parse_with(spec.reader(), &document.text, document.format)?;

    let (section, siblings) = sections::section_of(
        sections::table_of(parsed),
        sections::Layout::of(spec),
        spec.key,
    )?;

    into.document("remote", &Origin::Remote(store), section, siblings);

    Ok(())
}

fn collect_environment(
    into: &mut crate::resolve::Collected,
    spec: &LoadSpec<'_>,
) -> Result<(), Error> {
    let Some(prefix) = spec.full_env_prefix() else {
        return Ok(());
    };

    // The invariant a caller opted into is checked before the layer is
    // built, not after: an ambiguous spelling is refused whether or not the
    // value it holds would have won anything.
    if spec.strict_env {
        environment::reject_ambiguous(&prefix)?;
    }

    let tree = crate::env_layer::tree(&prefix, spec.nest, spec.allow_empty_env);

    if let crate::Value::Table(values) = tree {
        // Recorded even when it is empty: a configured prefix that supplied
        // nothing is an answer — `explain` prints it as `absent`, which is
        // how somebody finds out their variable is spelled wrong.
        into.layer(
            // The prefix, until `refine_env` narrows a leaf to the variable
            // that actually supplied it.
            "environment",
            Origin::Env(format!("{}*", prefix.to_ascii_uppercase())),
            values,
        );
    }

    Ok(())
}

fn collect_bindings(
    into: &mut crate::resolve::Collected,
    spec: &LoadSpec<'_>,
) -> Result<(), Error> {
    let Some(bindings) = spec.env_bindings else {
        return Ok(());
    };

    let fallback = env_file_entries(spec)?;

    for (path, variable, value) in bindings.resolved(spec.allow_empty_env, fallback) {
        let mut values = std::collections::BTreeMap::new();
        crate::layer::insert_path(&mut values, &path, value);

        into.layer("binding", Origin::Env(variable), values);
    }

    Ok(())
}

fn collect_runtime(into: &mut crate::resolve::Collected, spec: &LoadSpec<'_>) {
    if let Some(flags) = spec.flags {
        if !flags.is_empty() {
            into.layer("flag", Origin::Runtime("command-line flag"), flags.tree());
        }
    }

    if let Some(overrides) = spec.overrides {
        if !overrides.is_empty() {
            into.layer("override", Origin::Runtime("override"), overrides.tree());
        }
    }
}

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

use figment::providers::Env;
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
use figment::providers::Format as _;
#[cfg(feature = "json")]
use figment::providers::Json;
#[cfg(feature = "toml")]
use figment::providers::Toml;
#[cfg(feature = "yaml")]
use figment::providers::Yaml;
use figment::value::Dict;
use figment::{Figment, Metadata};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::error::{Error, ErrorKind, Origin};
use crate::layer::{DEFAULTS_NAME, FLAGS_NAME, OVERRIDES_NAME};
use crate::snapshot::Snapshot;
use crate::source::{Format, LoadSpec, Source};

/// How figment names the metadata of a provider built from a string.
const INLINE_SUFFIX: &str = "source string";

/// Metadata name for the recovery provider.
pub(crate) const CACHED_NAME: &str = "the last configuration that worked";

/// Prefixed onto a remote store's own description, so that a value traced back
/// to one names the store rather than reporting "an inline source" — which is
/// what figment sees, and is the wrong answer to the question being asked.
const REMOTE_PREFIX: &str = "the remote store ";

/// How figment names the metadata of its environment provider.
const ENV_SUFFIX: &str = "environment variable(s)";

pub(crate) fn load<T: DeserializeOwned>(spec: &LoadSpec<'_>) -> Result<T, Error> {
    apply_aliases(build(spec)?, spec)
        .select(spec.key)
        .extract()
        .map_err(convert)
}

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
) -> Result<T, Error> {
    let mut figment = Figment::new().merge(Cached {
        values: cached.values().clone(),
        profile: figment::Profile::from(spec.key),
    });

    if let Some(prefix) = spec.full_env_prefix() {
        figment = figment.merge(environment(
            &prefix,
            spec.key,
            spec.nest,
            spec.allow_empty_env,
        ));
    }

    figment = merge_env_files(figment, spec)?;

    if let Some(bindings) = spec.env_bindings {
        for binding in bindings.providers(spec.key) {
            figment = figment.merge(binding);
        }
    }

    if let Some(flags) = spec.flags {
        figment = figment.merge(flags.provider(spec.key, FLAGS_NAME));
    }

    if let Some(overrides) = spec.overrides {
        figment = figment.merge(overrides.provider(spec.key, OVERRIDES_NAME));
    }

    let figment = apply_aliases(figment, spec);

    figment.select(spec.key).extract().map_err(convert)
}

/// A provider that answers with someone else's data under its own name.
///
/// figment names a string provider `"JSON source string"` and gives it no
/// source, which is true and useless: two remote stores and a hand-written
/// literal are then indistinguishable in an error message.
///
/// Only reachable when a format is enabled — with no format at all there is
/// nothing to wrap.
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
struct Named<P> {
    inner: P,
    name: String,
    /// Set for a layer that really is a file — a decrypted one — so a value
    /// traced back to it reports `Origin::File` and not `Origin::Unknown`.
    file: Option<std::path::PathBuf>,
}

#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
impl<P: figment::Provider> figment::Provider for Named<P> {
    fn metadata(&self) -> Metadata {
        let metadata = Metadata::named(self.name.clone());

        match &self.file {
            Some(path) => metadata.source(figment::Source::File(path.clone())),
            None => metadata,
        }
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        self.inner.data()
    }

    fn profile(&self) -> Option<figment::Profile> {
        self.inner.profile()
    }
}

/// The cache, as a provider.
struct Cached {
    values: Dict,
    profile: figment::Profile,
}

impl figment::Provider for Cached {
    fn metadata(&self) -> Metadata {
        Metadata::named(CACHED_NAME)
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        let mut map = figment::value::Map::new();
        map.insert(self.profile.clone(), self.values.clone());

        Ok(map)
    }
}

/// Fills gaps from aliased paths.
///
/// Queried after everything else is merged, because that is the only point at
/// which "nothing supplies this" can be answered. An alias never overrides: a
/// file that has been updated wins over one that has not, whatever order they
/// merged in.
fn apply_aliases(figment: Figment, spec: &LoadSpec<'_>) -> Figment {
    let Some(aliases) = spec.aliases else {
        return figment;
    };

    let mut figment = figment;

    for (from, to) in aliases.pairs() {
        let selected = figment.clone().select(spec.key);

        if selected.find_value(&to).is_ok() {
            continue;
        }

        let Ok(value) = selected.find_value(&from) else {
            continue;
        };

        let mut values = Dict::new();
        crate::layer::insert_path(&mut values, &to, value);

        figment = figment.merge(Aliased {
            values,
            profile: figment::Profile::from(spec.key),
            from,
        });
    }

    figment
}

/// One aliased value, under the path the field actually has.
struct Aliased {
    values: Dict,
    profile: figment::Profile,
    from: String,
}

impl figment::Provider for Aliased {
    fn metadata(&self) -> Metadata {
        Metadata::named(format!("{ALIAS_PREFIX}{}", self.from))
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        let mut map = figment::value::Map::new();
        map.insert(self.profile.clone(), self.values.clone());

        Ok(map)
    }
}

/// Prefixed onto the old path in this provider's metadata name.
///
/// Not something `source_of` ever reports: figment attributes every path under
/// a section to whichever provider supplied the section, so an aliased value
/// traces back to the *file that holds the old spelling*. That is the more
/// useful answer anyway — it names the file to edit — and the name here still
/// shows up in figment's own messages.
const ALIAS_PREFIX: &str = "an alias for ";

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
        .map_or(Origin::Unknown, origin_of)
}

/// Where the value at `path` would come from, if anywhere.
pub(crate) fn source_of(spec: &LoadSpec<'_>, path: &str) -> Result<Option<Origin>, Error> {
    let figment = apply_aliases(build(spec)?, spec).select(spec.key);

    Ok(figment.find_metadata(path).map(origin_of))
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

    let profile = spec.profile();

    // Discovered files sit below the explicitly listed ones: `files = [..]` is
    // a deliberate statement, a search result is a guess about the machine.
    if let Some(search) = &spec.search {
        for (path, format) in search.resolve() {
            figment = merge_file(figment, &path, format)?;
            figment = merge_profile_variant(figment, &path, format, profile.as_deref())?;
        }
    }

    for source in spec.sources {
        figment = merge(figment, source)?;

        if let (Some(path), Some(format)) = (source.path(), source.format()) {
            figment = merge_profile_variant(figment, Path::new(path), format, profile.as_deref())?;
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

            figment = merge_named_text(figment, &document.text, document.format, &name, None)?;
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
        for binding in bindings.providers(spec.key) {
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

/// The environment provider for one section.
///
/// figment has no value-level filter, so dropping empty variables means finding
/// them first and filtering by name. The filter is installed *before* `split`,
/// so it sees the prefix-stripped name with its `__` separators intact — the
/// same shape `empty_keys` produces.
fn environment(prefix: &str, key: &str, nest: &str, allow_empty: bool) -> Env {
    let mut env = Env::prefixed(prefix);

    if !allow_empty {
        let empty = empty_keys(prefix);

        if !empty.is_empty() {
            env = env.filter_map(move |name| {
                let is_empty = empty
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(name.as_str()));

                (!is_empty).then(|| name.into())
            });
        }
    }

    env.split(nest).profile(key)
}

/// Prefix-stripped names of the variables under `prefix` whose value is blank.
fn empty_keys(prefix: &str) -> Vec<String> {
    std::env::vars()
        .filter(|(_, value)| value.trim().is_empty())
        .filter_map(|(name, _)| {
            name.get(..prefix.len())
                .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
                .map(|_| name[prefix.len()..].to_owned())
        })
        .collect()
}

/// Merges one source, choosing the provider from its format.
///
/// `.nested()` promotes each file's top-level keys to profiles, which is what
/// makes `select(key)` pick out one section and lets several config structs
/// share the same files.
fn merge(figment: Figment, source: &Source<'_>) -> Result<Figment, Error> {
    // A foreign provider is merged as figment sees it: this crate's mapping of
    // top-level keys to sections is a thing it does to *documents*, and a
    // provider hands over values that are already figment's.
    #[cfg(feature = "figment")]
    if let Some(provider) = source.foreign() {
        return Ok(figment.merge(Foreign(provider)));
    }

    // Every remaining kind parses text, so it has a format.
    let Some(format) = source.format() else {
        // Unreachable: only a provider has no format, and that returned above.
        return Ok(figment);
    };

    match source.path() {
        Some(path) if source.is_encrypted() => {
            merge_encrypted_file(figment, Path::new(path), format)
        }
        Some(path) => merge_file(figment, Path::new(path), format),
        None => merge_text(figment, source.inline_text().unwrap_or_default(), format),
    }
}

/// A borrowed provider, owned enough for `Figment::merge`.
///
/// `merge` wants something `Sized`; a `&dyn Provider` is not, and figment does
/// not implement `Provider` for references. Three delegating methods is the
/// whole of it.
#[cfg(feature = "figment")]
struct Foreign<'a>(&'a (dyn figment::Provider + Send + Sync));

#[cfg(feature = "figment")]
impl figment::Provider for Foreign<'_> {
    fn metadata(&self) -> Metadata {
        self.0.metadata()
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        self.0.data()
    }

    fn profile(&self) -> Option<figment::Profile> {
        self.0.profile()
    }
}

/// Reads an encrypted file, decrypts it, and merges the plaintext.
///
/// The layer carries the file's name rather than figment's `"JSON source
/// string"`, so a value traced back to it names the file it actually came from.
/// A missing file is skipped, exactly as an unencrypted one is: that is what
/// makes an optional `secrets.json.age` work.
fn merge_encrypted_file(figment: Figment, path: &Path, format: Format) -> Result<Figment, Error> {
    // Reachable only through the macro-free API: `#[dynamic_config]` turns a
    // `.age` file without the feature into a compile error naming it.
    #[cfg(not(feature = "decrypt"))]
    {
        let _ = (&figment, format);

        Err(Error::new(
            ErrorKind::Backend,
            format!(
                "{} is encrypted, and this build has no decryption support; \
                 add features = [\"age\"] to your dynamic-config dependency",
                path.display()
            ),
        ))
    }

    #[cfg(feature = "decrypt")]
    {
        let ciphertext = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(figment),
            Err(error) => {
                return Err(Error::new(ErrorKind::Io, error.to_string())
                    .with_origin(Origin::File(path.to_owned())))
            }
        };

        let named = path.display().to_string();
        let plaintext = crate::decrypt::decrypt(&ciphertext, &named)?;

        merge_named_text(figment, plaintext.text(), format, &named, Some(path))
    }
}

/// Layers `config.{profile}.toml` over `config.toml`.
///
/// The variant is merged unconditionally: figment treats a file that is not
/// there as an empty provider, so there is nothing to check first and no race
/// between checking and reading.
fn merge_profile_variant(
    figment: Figment,
    path: &Path,
    format: Format,
    profile: Option<&str>,
) -> Result<Figment, Error> {
    let Some(profile) = profile else {
        return Ok(figment);
    };

    let Some(variant) = profile_variant(path, profile) else {
        return Ok(figment);
    };

    if variant
        .to_str()
        .and_then(crate::source::inner_name)
        .is_some()
    {
        return merge_encrypted_file(figment, &variant, format);
    }

    merge_file(figment, &variant, format)
}

/// `config.toml` + `production` → `config.production.toml`.
fn profile_variant(path: &Path, profile: &str) -> Option<PathBuf> {
    // An encrypted file's real extension is `.age`, so the profile goes under
    // it: `secrets.json.age` becomes `secrets.production.json.age`, not
    // `secrets.json.production.age`, which is what naming the suffix would give.
    if let Some((inner, _)) = path
        .to_str()
        .and_then(|path| crate::source::inner_name(path))
    {
        let variant = profile_variant(Path::new(inner), profile)?;

        return Some(PathBuf::from(format!(
            "{}.{}",
            variant.display(),
            crate::source::ENCRYPTED_SUFFIX
        )));
    }

    let stem = path.file_stem()?.to_str()?;
    let extension = path.extension()?.to_str()?;

    Some(path.with_file_name(format!("{stem}.{profile}.{extension}")))
}

fn merge_file(figment: Figment, path: &Path, format: Format) -> Result<Figment, Error> {
    // With no format feature on, every arm below is compiled out.
    #[cfg(not(any(feature = "json", feature = "toml", feature = "yaml")))]
    let _ = (&figment, path);

    match format {
        #[cfg(feature = "json")]
        Format::Json => Ok(figment.merge(Sections::from(Json::file(path)))),
        #[cfg(feature = "toml")]
        Format::Toml => Ok(figment.merge(Sections::from(Toml::file(path)))),
        #[cfg(feature = "yaml")]
        Format::Yaml => Ok(figment.merge(Sections::from(Yaml::file(path)))),

        #[allow(unreachable_patterns)]
        format => Err(disabled(format)),
    }
}

/// The key an editor uses to find a schema, tolerated at the top level.
///
/// Every other top-level key is a section, so a bare string there is an error.
/// This one is the exception because it is how JSON says "here is my schema",
/// and refusing it would mean the schema this crate can emit could not be wired
/// up in the file it describes.
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
const SCHEMA_KEY: &str = "$schema";

/// Top-level keys as profiles, which is what makes a key a *section*.
///
/// This is what figment's `nested()` does, reimplemented for two reasons:
/// [`SCHEMA_KEY`] has to survive, and a top-level key that is not a table
/// deserves an error that names it. figment reports `invalid type: string,
/// expected a map` with an offset, which is true and leaves the reader to work
/// out that the crate treats top-level keys as sections.
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
struct Sections<P> {
    inner: P,
}

#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
impl<P: figment::Provider> From<P> for Sections<P> {
    fn from(inner: P) -> Self {
        Self { inner }
    }
}

#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
impl<P: figment::Provider> figment::Provider for Sections<P> {
    fn metadata(&self) -> Metadata {
        self.inner.metadata()
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        let mut sections = figment::value::Map::new();

        // The inner provider is *not* nested, so it answers with one profile
        // holding the whole document.
        for (_, document) in self.inner.data()? {
            for (key, value) in document {
                if key == SCHEMA_KEY {
                    continue;
                }

                let figment::value::Value::Dict(_, dict) = value else {
                    return Err(figment::Error::from(format!(
                        "top-level key `{key}` is not a table; every top-level key \
                         in a config file is a section, so a value there must be a \
                         table (`{SCHEMA_KEY}` is the one exception)"
                    )));
                };

                sections.insert(figment::Profile::from(key), dict);
            }
        }

        Ok(sections)
    }
}

fn merge_text(figment: Figment, text: &str, format: Format) -> Result<Figment, Error> {
    #[cfg(not(any(feature = "json", feature = "toml", feature = "yaml")))]
    let _ = (&figment, text);

    match format {
        #[cfg(feature = "json")]
        Format::Json => Ok(figment.merge(Sections::from(Json::string(text)))),
        #[cfg(feature = "toml")]
        Format::Toml => Ok(figment.merge(Sections::from(Toml::string(text)))),
        #[cfg(feature = "yaml")]
        Format::Yaml => Ok(figment.merge(Sections::from(Yaml::string(text)))),

        #[allow(unreachable_patterns)]
        format => Err(disabled(format)),
    }
}

/// As [`merge_text`], but the layer carries `name` instead of figment's
/// `"JSON source string"`.
fn merge_named_text(
    figment: Figment,
    text: &str,
    format: Format,
    name: &str,
    file: Option<&Path>,
) -> Result<Figment, Error> {
    #[cfg(not(any(feature = "json", feature = "toml", feature = "yaml")))]
    let _ = (&figment, text, name, file);

    // A generic fn rather than a closure: a closure infers one provider type
    // from its first call and the other two formats then fail to compile.
    #[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
    fn named<P>(inner: P, name: &str, file: Option<&Path>) -> Named<P> {
        Named {
            inner,
            name: name.to_owned(),
            file: file.map(Path::to_owned),
        }
    }

    match format {
        #[cfg(feature = "json")]
        Format::Json => Ok(figment.merge(named(Sections::from(Json::string(text)), name, file))),
        #[cfg(feature = "toml")]
        Format::Toml => Ok(figment.merge(named(Sections::from(Toml::string(text)), name, file))),
        #[cfg(feature = "yaml")]
        Format::Yaml => Ok(figment.merge(named(Sections::from(Yaml::string(text)), name, file))),

        #[allow(unreachable_patterns)]
        format => Err(disabled(format)),
    }
}

/// Unreachable through `#[dynamic_config]`, which turns a disabled format into a
/// compile error naming the feature. Reachable by hand, so it says the same
/// thing at runtime.
fn disabled(format: Format) -> Error {
    Error::new(
        ErrorKind::Backend,
        format!(
            "cannot read {format:?} because the `{}` feature is not enabled; \
             add features = [\"{}\"] to your dynamic-config dependency",
            format.feature(),
            format.feature(),
        ),
    )
}

/// The message for a figment error, with any offending *value* left out.
///
/// figment renders a type mismatch as ``invalid type: found string "hunter2",
/// expected u16``, which is helpful right up until the value is a secret — a
/// password pasted into a numeric field lands in a log line, and every other
/// diagnostic in this crate goes to some length to make sure that cannot
/// happen. The key path, the kind of thing that was there, and the type that
/// was wanted are all kept; only the value goes.
fn message(error: &figment::Error) -> String {
    use figment::error::Kind;

    match &error.kind {
        Kind::InvalidType(actual, expected) => {
            format!(
                "invalid type: found {}, expected {expected}",
                kind_of(actual)
            )
        }
        Kind::InvalidValue(actual, expected) => {
            format!(
                "invalid value: found {}, expected {expected}",
                kind_of(actual)
            )
        }
        _ => error.to_string(),
    }
}

/// Names what was there without saying what it was.
///
/// The variants that carry no payload keep figment's own wording; the ones that
/// do are reduced to their type. A length is not a secret, so
/// `InvalidLength` is left alone above.
fn kind_of(actual: &figment::error::Actual) -> &'static str {
    use figment::error::Actual;

    match actual {
        Actual::Bool(_) => "a boolean",
        Actual::Unsigned(_) => "an unsigned integer",
        Actual::Signed(_) => "a signed integer",
        Actual::Float(_) => "a float",
        Actual::Char(_) => "a character",
        Actual::Str(_) => "a string",
        Actual::Bytes(_) => "a byte string",
        Actual::Unit => "a unit",
        Actual::Option => "an option",
        Actual::NewtypeStruct => "a newtype struct",
        Actual::Seq => "a list",
        Actual::Map => "a table",
        Actual::Enum => "an enum",
        Actual::UnitVariant => "a unit variant",
        Actual::NewtypeVariant => "a newtype variant",
        Actual::TupleVariant => "a tuple variant",
        Actual::StructVariant => "a struct variant",
        // `Other` is a free-form description rather than a value, but it comes
        // from whatever produced the error, so it is not ours to vouch for.
        Actual::Other(_) => "something else",
    }
}

/// Merges every `.env` file, in order, below the real environment.
///
/// Nothing at all without an `env` prefix: a `.env` holds variable names, and
/// without a prefix there is no rule for which of them belong to this section.
#[cfg(feature = "dotenv")]
fn merge_env_files(mut figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    let Some(prefix) = spec.full_env_prefix() else {
        return Ok(figment);
    };

    for file in spec.env_files {
        let path = Path::new(file);
        let entries = crate::dotenv::read(path)?;

        if entries.is_empty() {
            continue;
        }

        figment = figment.merge(crate::dotenv::DotenvProvider::new(
            entries,
            path,
            &prefix,
            spec.key,
            spec.nest,
            spec.allow_empty_env,
        ));
    }

    Ok(figment)
}

/// Reachable only through the macro-free API: `#[dynamic_config]` turns
/// `env_files` without the feature into a compile error naming it.
#[cfg(not(feature = "dotenv"))]
fn merge_env_files(figment: Figment, spec: &LoadSpec<'_>) -> Result<Figment, Error> {
    if spec.env_files.is_empty() {
        return Ok(figment);
    }

    Err(Error::new(
        ErrorKind::Backend,
        "`.env` files need the `dotenv` feature; add features = [\"dotenv\"] \
         to your dynamic-config dependency",
    ))
}

/// Translates a figment error, preserving the key path and the source.
fn convert(error: figment::Error) -> Error {
    use figment::error::Kind;

    let kind = match &error.kind {
        Kind::MissingField(_) => ErrorKind::Missing,
        Kind::InvalidType(..)
        | Kind::InvalidValue(..)
        | Kind::InvalidLength(..)
        | Kind::ISizeOutOfRange(_)
        | Kind::USizeOutOfRange(_) => ErrorKind::Type,
        // figment has no dedicated parse variant: a provider that fails to read
        // its document reports a bare `Message` with no key path, whereas a
        // serde error raised during extraction always carries one.
        Kind::Message(_) if error.path.is_empty() => ErrorKind::Parse,
        Kind::Message(_) => ErrorKind::Type,
        _ => ErrorKind::Backend,
    };

    // A missing field is named inside the kind rather than in the path, so it
    // has to be moved across for `Error::path()` to be useful.
    let mut path = error.path.clone();

    if path.is_empty() {
        if let Kind::MissingField(field) = &error.kind {
            path.push(field.to_string());
        }
    }

    let origin = error.metadata.as_ref().map_or(Origin::Unknown, origin_of);
    let mut translated = Error::new(kind, message(&error)).with_origin(origin);

    // Rebuilt outermost-last so `Error::path()` reads root-first.
    for segment in path.into_iter().rev() {
        translated = translated.prepend_key(segment);
    }

    translated
}

/// Pulls the prefix out of ``"`APP_DB_` environment variable(s)"``.
///
/// figment names the environment provider after the prefix it was built with,
/// which is as specific as it gets: the provider knows the prefix, not which
/// variable under it failed.
fn env_prefix(name: &str) -> String {
    let prefix = name.trim_end_matches(ENV_SUFFIX).trim().trim_matches('`');

    if prefix.is_empty() {
        return "the environment".to_owned();
    }

    format!("{prefix}*")
}

fn origin_of(metadata: &Metadata) -> Origin {
    // The runtime layers are named by us, so they are recognised by name rather
    // than by source — figment models both them and the environment as
    // `Source::Custom`.
    if metadata.name == DEFAULTS_NAME {
        return Origin::Runtime("default");
    }

    if metadata.name == OVERRIDES_NAME {
        return Origin::Runtime("override");
    }

    if metadata.name == FLAGS_NAME {
        return Origin::Runtime("command-line flag");
    }

    if metadata.name == CACHED_NAME {
        return Origin::Runtime("cached configuration");
    }

    // A binding names its own variable, which is the answer to the question
    // being asked — "where did this come from" is not usefully answered with
    // "a binding".
    if let Some(variable) = metadata.name.strip_prefix(crate::bindings::BINDING_PREFIX) {
        return Origin::Env(variable.to_owned());
    }

    #[cfg(feature = "dotenv")]
    if let Some(file) = metadata.name.strip_prefix(crate::dotenv::PREFIX) {
        return Origin::File(std::path::PathBuf::from(file));
    }

    if let Some(store) = metadata.name.strip_prefix(REMOTE_PREFIX) {
        return Origin::Remote(store.to_owned());
    }

    match metadata.source.as_ref() {
        Some(figment::Source::File(path)) => Origin::File(path.clone()),
        // figment names the env provider by its prefix rather than by the
        // variable that failed, so this is as specific as it gets.
        Some(figment::Source::Custom(name)) => Origin::Env(name.clone()),
        Some(_) => Origin::Inline,
        // Providers that read no file carry no source at all, only a name:
        // ``"`APP_DB_` environment variable(s)"``, `"JSON source string"`.
        // Recognising them by name is brittle by nature, which is why
        // `tests/loader.rs` asserts it rather than leaving it to be noticed in
        // a bug report.
        None if metadata.name.ends_with(ENV_SUFFIX) => Origin::Env(env_prefix(&metadata.name)),
        None if metadata.name.ends_with(INLINE_SUFFIX) => Origin::Inline,
        None => Origin::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_variant_sits_next_to_its_base() {
        assert_eq!(
            profile_variant(Path::new("/etc/app/config.toml"), "production"),
            Some(PathBuf::from("/etc/app/config.production.toml"))
        );
        assert_eq!(
            profile_variant(Path::new("config.json"), "dev"),
            Some(PathBuf::from("config.dev.json"))
        );
    }

    #[test]
    fn the_environment_prefix_is_recovered_from_figments_name() {
        assert_eq!(env_prefix("`APP_DB_` environment variable(s)"), "APP_DB_*");
        assert_eq!(env_prefix("environment variable(s)"), "the environment");
    }

    #[test]
    fn a_path_without_an_extension_has_no_variant() {
        assert_eq!(profile_variant(Path::new("config"), "production"), None);
    }
}

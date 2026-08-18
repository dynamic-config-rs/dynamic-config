//! Providers and merge helpers: how one source becomes one figment layer,
//! and how a profile picks the sibling file that overlays it.

#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
use figment::providers::Format as _;
#[cfg(feature = "json")]
use figment::providers::Json;
#[cfg(feature = "toml")]
use figment::providers::Toml;
#[cfg(feature = "yaml")]
use figment::providers::Yaml;

#[cfg(feature = "ini")]
use super::ini::Ini;
#[cfg(feature = "properties")]
use super::properties::Properties;
use figment::value::Dict;
use figment::{Figment, Metadata};
use std::path::{Path, PathBuf};

use super::CACHED_NAME;
#[cfg(feature = "decrypt")]
use crate::error::Origin;
use crate::error::{Error, ErrorKind};
use crate::source::{Format, LoadSpec, Source};

/// A provider that answers with someone else's data under its own name.
///
/// figment names a string provider `"JSON source string"` and gives it no
/// source, which is true and useless: two remote stores and a hand-written
/// literal are then indistinguishable in an error message.
///
/// Only reachable when a format is enabled — with no format at all there is
/// nothing to wrap.
#[cfg(any(
    feature = "json",
    feature = "toml",
    feature = "yaml",
    feature = "ini",
    feature = "properties"
))]
struct Named<P> {
    inner: P,
    name: String,
    /// Set for a layer that really is a file — a decrypted one — so a value
    /// traced back to it reports `Origin::File` and not `Origin::Unknown`.
    file: Option<std::path::PathBuf>,
}

#[cfg(any(
    feature = "json",
    feature = "toml",
    feature = "yaml",
    feature = "ini",
    feature = "properties"
))]
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
pub(super) struct Cached {
    pub(super) values: Dict,
    pub(super) profile: figment::Profile,
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

/// Merges one source, choosing the provider from its format.
///
/// `.nested()` promotes each file's top-level keys to profiles, which is what
/// makes `select(key)` pick out one section and lets several config structs
/// share the same files.
pub(super) fn merge(
    figment: Figment,
    source: &Source<'_>,
    layout: Layout<'_>,
) -> Result<Figment, Error> {
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
            merge_encrypted_file(figment, Path::new(path), format, layout)
        }
        Some(path) => merge_file(figment, Path::new(path), format, layout),
        None => merge_text(
            figment,
            source.inline_text().unwrap_or_default(),
            format,
            layout,
        ),
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
        // The documented contract stands — a provider produces its section
        // as a profile named after the section — and the loader's internal
        // namespacing is applied *for* it here, so the contract survives
        // the prefix. `default` and `global` pass through untouched: for a
        // provider author they are figment's own vocabulary, deliberately
        // reachable through this one door.
        Ok(self
            .0
            .data()?
            .into_iter()
            .map(|(profile, dict)| {
                if profile == figment::Profile::Default || profile == figment::Profile::Global {
                    (profile, dict)
                } else {
                    (
                        figment::Profile::from(super::section_profile(profile.as_str().as_str())),
                        dict,
                    )
                }
            })
            .collect())
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
fn merge_encrypted_file(
    figment: Figment,
    path: &Path,
    format: Format,
    layout: Layout<'_>,
) -> Result<Figment, Error> {
    // Reachable only through the macro-free API: `#[dynamic_config]` turns a
    // `.age` file without the feature into a compile error naming it.
    #[cfg(not(feature = "decrypt"))]
    {
        let _ = (&figment, format, layout);

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

        merge_named_text(
            figment,
            plaintext.text(),
            format,
            &named,
            Some(path),
            layout,
        )
    }
}

/// Layers `config.{profile}.toml` over `config.toml`.
///
/// The variant is merged unconditionally: figment treats a file that is not
/// there as an empty provider, so there is nothing to check first and no race
/// between checking and reading.
pub(super) fn merge_profile_variant(
    figment: Figment,
    path: &Path,
    format: Format,
    profile: Option<&str>,
    layout: Layout<'_>,
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
        return merge_encrypted_file(figment, &variant, format, layout);
    }

    merge_file(figment, &variant, format, layout)
}

/// The active profile, refused unless it can only ever name a sibling file.
///
/// The profile is interpolated into a *file name* — `config.{profile}.toml` —
/// and it arrives from an environment variable. `APP_ENV=../../tmp/evil`
/// would otherwise walk the loader to an arbitrary path and merge whatever
/// it finds *above* the base file. A profile is a word like `production`;
/// anything with a path separator or a parent reference is an attack or an
/// accident, and both deserve an error naming the variable.
pub(super) fn validated_profile(spec: &LoadSpec<'_>) -> Result<Option<String>, Error> {
    let Some(profile) = spec.profile() else {
        return Ok(None);
    };

    if profile_is_safe(&profile) {
        return Ok(Some(profile));
    }

    Err(Error::new(
        ErrorKind::Env,
        format!(
            "`{}` names the active profile and becomes part of a file name; \
             a profile must be a plain word, not a path",
            spec.profile_variable().unwrap_or("the profile variable"),
        ),
    ))
}

/// Whether a profile can only ever name a sibling of the file it is applied to.
///
/// Split out of [`validated_profile`] so the rule can be checked against
/// [`profile_variant`] directly — the two together are the whole of the 0.4
/// traversal fix, and a fuzzer reaches them through
/// [`__fuzz`](crate::__fuzz).
pub(crate) fn profile_is_safe(profile: &str) -> bool {
    !profile.contains('/')
        && !profile.contains('\\')
        && !profile.contains("..")
        && !profile.contains('\0')
}

/// `config.toml` + `production` → `config.production.toml`.
///
/// The directory is never touched. Everything below works on the *file name*
/// and the result is rejoined to the original path exactly once, which is
/// what makes the sibling rule structural rather than something each branch
/// has to remember — see [`name_variant`].
pub(crate) fn profile_variant(path: &Path, profile: &str) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;

    Some(path.with_file_name(name_variant(name, profile)?))
}

/// The naming half of [`profile_variant`], on a file name alone.
///
/// It takes a `&str` rather than a `&Path` on purpose. Reasoning about a
/// whole path here is what broke the sibling rule once already: stripping
/// `.age` off `dir.d/..age` leaves `dir.d/.`, whose *file name* is `dir.d` —
/// so a recursion on the path renamed the **directory** and the variant
/// landed one level up, in a directory the caller never named. A fuzz target
/// found it (`fuzz/fuzz_targets/sections.rs`); a name cannot express a
/// directory, so the shape that produced it is gone rather than guarded.
fn name_variant(name: &str, profile: &str) -> Option<String> {
    // An encrypted file's real extension is `.age`, so the profile goes under
    // it: `secrets.json.age` becomes `secrets.production.json.age`, not
    // `secrets.json.production.age`, which is what naming the suffix would give.
    if let Some((inner, _)) = crate::source::inner_name(name) {
        let inner = name_variant(inner, profile)?;

        return Some(format!("{inner}.{}", crate::source::ENCRYPTED_SUFFIX));
    }

    let name = Path::new(name);
    let stem = name.file_stem()?.to_str()?;
    let extension = name.extension()?.to_str()?;

    Some(format!("{stem}.{profile}.{extension}"))
}

pub(super) fn merge_file(
    figment: Figment,
    path: &Path,
    format: Format,
    layout: Layout<'_>,
) -> Result<Figment, Error> {
    // With no format feature on, every arm below is compiled out.
    #[cfg(not(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "ini",
        feature = "properties"
    )))]
    let _ = (&figment, path, layout);

    match format {
        #[cfg(feature = "json")]
        Format::Json => Ok(figment.merge(Sections::new(Json::file(path), layout))),
        #[cfg(feature = "toml")]
        Format::Toml => Ok(figment.merge(Sections::new(Toml::file(path), layout))),
        #[cfg(feature = "yaml")]
        Format::Yaml => Ok(figment.merge(Sections::new(Yaml::file(path), layout))),
        #[cfg(feature = "ini")]
        Format::Ini => Ok(figment.merge(Sections::new(Ini::file(path), layout))),
        #[cfg(feature = "properties")]
        Format::Properties => Ok(figment.merge(Sections::new(Properties::file(path), layout))),

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
#[cfg(any(
    feature = "json",
    feature = "toml",
    feature = "yaml",
    feature = "ini",
    feature = "properties"
))]
const SCHEMA_KEY: &str = "$schema";

/// Where one document's values live: under section headers, or at its root.
///
/// One value threaded through every merge rather than a second set of
/// functions, because a load reads listed files, discovered files, profile
/// variants, an encrypted file and a remote store's document — and if they
/// did not all agree about their own shape, a configuration assembled from
/// two of them would mean two different things.
#[derive(Clone, Copy)]
pub(super) struct Layout<'a> {
    /// `Some(key)` when the documents carry no section header, and all of
    /// each one is `key`'s values; `None` for the default, where every
    /// top-level key names a section.
    ///
    /// Read by [`Sections`], which is the one thing that reads a document —
    /// and which is compiled out entirely when no format feature is on. A
    /// build with no parser still *carries* the layout, because every merge
    /// signature takes one; there is simply nothing left to apply it to.
    #[cfg_attr(
        not(any(feature = "json", feature = "toml", feature = "yaml")),
        allow(dead_code)
    )]
    whole: Option<&'a str>,
}

impl<'a> Layout<'a> {
    /// What `spec` says its documents look like.
    pub(super) fn of(spec: &LoadSpec<'a>) -> Self {
        Self {
            whole: spec.whole_document.then_some(spec.key),
        }
    }
}

/// Top-level keys as profiles, which is what makes a key a *section*.
///
/// This is what figment's `nested()` does, reimplemented for three reasons:
/// [`SCHEMA_KEY`] has to survive; a top-level key that is not a table
/// deserves an error that names it — figment reports `invalid type: string,
/// expected a map` with an offset, which is true and leaves the reader to
/// work out that the crate treats top-level keys as sections — and a
/// [`Layout`] with no headers has to file the whole document under one
/// profile instead of looking for them.
#[cfg(any(
    feature = "json",
    feature = "toml",
    feature = "yaml",
    feature = "ini",
    feature = "properties"
))]
struct Sections<'a, P> {
    inner: P,
    layout: Layout<'a>,
}

#[cfg(any(
    feature = "json",
    feature = "toml",
    feature = "yaml",
    feature = "ini",
    feature = "properties"
))]
impl<'a, P: figment::Provider> Sections<'a, P> {
    const fn new(inner: P, layout: Layout<'a>) -> Self {
        Self { inner, layout }
    }
}

#[cfg(any(
    feature = "json",
    feature = "toml",
    feature = "yaml",
    feature = "ini",
    feature = "properties"
))]
impl<P: figment::Provider> figment::Provider for Sections<'_, P> {
    fn metadata(&self) -> Metadata {
        self.inner.metadata()
    }

    fn data(&self) -> figment::Result<figment::value::Map<figment::Profile, Dict>> {
        let mut sections = figment::value::Map::new();

        // The inner provider is *not* nested, so it answers with one profile
        // holding the whole document.
        for (_, document) in self.inner.data()? {
            // A document with no section header *is* one section's values,
            // so it is filed whole and nothing is assumed about its
            // top-level keys — `{"host": "0.0.0.0", "port": 8000}` is
            // exactly the shape the sectioned reading below has to refuse.
            if let Some(key) = self.layout.whole {
                let mut values = document;
                values.remove(SCHEMA_KEY);

                sections.insert(figment::Profile::from(super::section_profile(key)), values);

                continue;
            }

            for (key, value) in document {
                if key == SCHEMA_KEY {
                    continue;
                }

                let figment::value::Value::Dict(_, dict) = value else {
                    return Err(figment::Error::from(format!(
                        "top-level key `{key}` is not a table; every top-level key \
                         in a config file is a section, so a value there must be a \
                         table (`{SCHEMA_KEY}` is the one exception). If this file \
                         is not sectioned — if the whole of it is one \
                         configuration — read it with `.whole_document()`"
                    )));
                };

                sections.insert(figment::Profile::from(super::section_profile(&key)), dict);
            }
        }

        Ok(sections)
    }
}

fn merge_text(
    figment: Figment,
    text: &str,
    format: Format,
    layout: Layout<'_>,
) -> Result<Figment, Error> {
    #[cfg(not(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "ini",
        feature = "properties"
    )))]
    let _ = (&figment, text, layout);

    match format {
        #[cfg(feature = "json")]
        Format::Json => Ok(figment.merge(Sections::new(Json::string(text), layout))),
        #[cfg(feature = "toml")]
        Format::Toml => Ok(figment.merge(Sections::new(Toml::string(text), layout))),
        #[cfg(feature = "yaml")]
        Format::Yaml => Ok(figment.merge(Sections::new(Yaml::string(text), layout))),
        #[cfg(feature = "ini")]
        Format::Ini => Ok(figment.merge(Sections::new(Ini::string(text), layout))),
        #[cfg(feature = "properties")]
        Format::Properties => Ok(figment.merge(Sections::new(Properties::string(text), layout))),

        #[allow(unreachable_patterns)]
        format => Err(disabled(format)),
    }
}

/// As [`merge_text`], but the layer carries `name` instead of figment's
/// `"JSON source string"`.
pub(super) fn merge_named_text(
    figment: Figment,
    text: &str,
    format: Format,
    name: &str,
    file: Option<&Path>,
    layout: Layout<'_>,
) -> Result<Figment, Error> {
    #[cfg(not(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "ini",
        feature = "properties"
    )))]
    let _ = (&figment, text, name, file, layout);

    // A generic fn rather than a closure: a closure infers one provider type
    // from its first call and the other two formats then fail to compile.
    #[cfg(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "ini",
        feature = "properties"
    ))]
    fn named<P>(inner: P, name: &str, file: Option<&Path>) -> Named<P> {
        Named {
            inner,
            name: name.to_owned(),
            file: file.map(Path::to_owned),
        }
    }

    match format {
        #[cfg(feature = "json")]
        Format::Json => {
            Ok(figment.merge(named(Sections::new(Json::string(text), layout), name, file)))
        }
        #[cfg(feature = "toml")]
        Format::Toml => {
            Ok(figment.merge(named(Sections::new(Toml::string(text), layout), name, file)))
        }
        #[cfg(feature = "yaml")]
        Format::Yaml => {
            Ok(figment.merge(named(Sections::new(Yaml::string(text), layout), name, file)))
        }
        #[cfg(feature = "ini")]
        Format::Ini => {
            Ok(figment.merge(named(Sections::new(Ini::string(text), layout), name, file)))
        }
        #[cfg(feature = "properties")]
        Format::Properties => Ok(figment.merge(named(
            Sections::new(Properties::string(text), layout),
            name,
            file,
        ))),

        #[allow(unreachable_patterns)]
        format => Err(disabled(format)),
    }
}

/// One document, parsed into its tree, with no section mapping applied.
///
/// The other direction from [`crate::write`]'s `render`, and the half of the
/// parse seam that [`Value::parse`](crate::Value::parse) is: a caller outside
/// this crate that wants to *merge* documents before the loader sees them
/// needs the tree, not the profiles the loader files it under. The `Sections`
/// wrapper is deliberately absent — it turns top-level keys into profiles, and
/// a merge of several documents happens below that line.
///
/// Errors travel through [`origin::translate`](super::origin::translate) like
/// every other figment failure here, so the offending value is stripped on this
/// road too.
pub(crate) fn parse_document(text: &str, format: Format) -> Result<Dict, Error> {
    #[cfg(not(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "ini",
        feature = "properties"
    )))]
    let _ = text;

    // `Provider::data`, not `Figment::extract`: a figment would re-merge and
    // re-tag what has just been parsed, for a tree that is thrown away again
    // as soon as it becomes a `crate::Value`. A string provider that was never
    // `nested()` files the whole document under the default profile, so there
    // is exactly one entry to take.
    #[cfg(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "ini",
        feature = "properties"
    ))]
    fn document<P: figment::Provider>(provider: P) -> Result<Dict, Error> {
        Ok(provider
            .data()
            .map_err(|error| super::origin::translate(&error))?
            .remove(&figment::Profile::Default)
            .unwrap_or_default())
    }

    match format {
        #[cfg(feature = "json")]
        Format::Json => document(Json::string(text)),
        #[cfg(feature = "toml")]
        Format::Toml => document(Toml::string(text)),
        #[cfg(feature = "yaml")]
        Format::Yaml => document(Yaml::string(text)),
        #[cfg(feature = "ini")]
        Format::Ini => document(Ini::string(text)),
        #[cfg(feature = "properties")]
        Format::Properties => document(Properties::string(text)),

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
    fn a_path_without_an_extension_has_no_variant() {
        assert_eq!(profile_variant(Path::new("config"), "production"), None);
    }

    #[test]
    fn an_encrypted_file_carries_the_profile_under_its_suffix() {
        assert_eq!(
            profile_variant(Path::new("/etc/app/secrets.json.age"), "production"),
            Some(PathBuf::from("/etc/app/secrets.production.json.age"))
        );
    }

    /// A directory with a dot in its name is ordinary — `/etc/my.app`,
    /// `/srv/conf.d` — and a file called `..age` inside one used to walk the
    /// variant *out* of that directory: `/etc/my.app/..age` produced
    /// `/etc/my.production.app.age`, one level up, which is exactly the
    /// traversal the profile rule exists to prevent. Found by
    /// `fuzz/fuzz_targets/sections.rs`.
    #[test]
    fn a_variant_never_leaves_the_directory_it_was_built_from() {
        for base in [
            "/etc/my.app/..age",
            "/srv/conf.d/..age",
            "/etc/my.app/config.toml",
            "relative.d/..age",
        ] {
            let base = Path::new(base);
            let Some(variant) = profile_variant(base, "production") else {
                continue;
            };

            assert_eq!(
                variant.parent(),
                base.parent(),
                "{} moved to {}",
                base.display(),
                variant.display()
            );
        }
    }
}

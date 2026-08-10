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
use figment::value::Dict;
use figment::{Figment, Metadata};
use std::path::{Path, PathBuf};

#[cfg(feature = "decrypt")]
use crate::error::Origin;
use crate::error::{Error, ErrorKind};
use crate::source::{Format, LoadSpec, Source};

use super::CACHED_NAME;

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
pub(super) fn merge(figment: Figment, source: &Source<'_>) -> Result<Figment, Error> {
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
pub(super) fn merge_profile_variant(
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

    let clean = !profile.contains('/')
        && !profile.contains('\\')
        && !profile.contains("..")
        && !profile.contains('\0');

    if clean {
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

pub(super) fn merge_file(figment: Figment, path: &Path, format: Format) -> Result<Figment, Error> {
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
pub(super) fn merge_named_text(
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
}

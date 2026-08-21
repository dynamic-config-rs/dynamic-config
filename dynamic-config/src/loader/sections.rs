//! How one source becomes one layer's contribution, and how a profile picks
//! the sibling file that overlays it.

use std::path::{Path, PathBuf};

use crate::error::Origin;
use crate::error::{Error, ErrorKind};
use crate::source::{Format, LoadSpec, Source};

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

/// A parsed document as a table. Parsing already refused anything else.
pub(super) fn table_of(document: crate::Value) -> Table {
    match document {
        crate::Value::Table(table) => table,
        _ => Table::new(),
    }
}

/// The key an editor uses to find a schema, tolerated at the top level.
///
/// Every other top-level key is a section, so a bare string there is an error.
/// This one is the exception because it is how JSON says "here is my schema",
/// and refusing it would mean the schema this crate can emit could not be wired
/// up in the file it describes.
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
    /// Which parser reads them. Here beside the shape because it is the
    /// other half of the same question — a document is read by a parser
    /// *and* narrowed by a layout, and every collector needs both.
    reader: &'static dyn crate::reader::Reader,
}

impl<'a> Layout<'a> {
    /// What `spec` says its documents look like.
    pub(super) fn of(spec: &LoadSpec<'a>) -> Self {
        Self {
            whole: spec.whole_document.then_some(spec.key),
            reader: spec.reader(),
        }
    }

    /// The parser this load's documents go through.
    pub(super) fn reader(self) -> &'static dyn crate::reader::Reader {
        self.reader
    }
}

/// A parsed document: keys at the top, values below.
type Table = std::collections::BTreeMap<String, crate::Value>;

/// The section a file has to say about, as a contribution.
///
/// An absent file contributes nothing, and so does a file that holds only
/// other people's sections.
pub(super) fn collect_file(
    into: &mut crate::resolve::Collected,
    layer: &'static str,
    path: &Path,
    format: Format,
    layout: Layout<'_>,
    key: &str,
) -> Result<(), Error> {
    let Some(document) = crate::document::read(layout.reader(), path, format)? else {
        return Ok(());
    };

    let (section, siblings) = section_of(table_of(document), layout, key)?;
    into.document(layer, &Origin::File(path.to_owned()), section, siblings);

    Ok(())
}

/// The sibling a profile selects — `config.production.toml` beside
/// `config.toml` — read the same way and layered over it.
pub(super) fn collect_profile_variant(
    into: &mut crate::resolve::Collected,
    layer: &'static str,
    path: &Path,
    format: Format,
    profile: Option<&str>,
    layout: Layout<'_>,
    key: &str,
) -> Result<(), Error> {
    let Some(profile) = profile else {
        return Ok(());
    };

    let Some(variant) = profile_variant(path, profile) else {
        return Ok(());
    };

    // `config.production.json.age` is a variant of an encrypted file and is
    // still encrypted: reading it as text would hand a parser a stream of
    // ciphertext.
    if variant
        .to_str()
        .and_then(crate::source::inner_name)
        .is_some()
    {
        return collect_encrypted_file(into, layer, &variant, format, layout, key);
    }

    collect_file(into, layer, &variant, format, layout, key)
}

/// One encrypted file's contribution: read, decrypt, parse, narrow.
///
/// A missing file is skipped exactly as an unencrypted one is — that is what
/// makes an optional `secrets.json.age` work — and the layer carries the
/// file's own name, so a value traced back to it names the file rather than
/// the plaintext it briefly was.
fn collect_encrypted_file(
    into: &mut crate::resolve::Collected,
    layer: &'static str,
    path: &Path,
    format: Format,
    layout: Layout<'_>,
    key: &str,
) -> Result<(), Error> {
    // Reachable only through the macro-free API: `#[dynamic_config]` turns a
    // `.age` file without the feature into a compile error naming it.
    #[cfg(not(feature = "decrypt"))]
    {
        let _ = (into, layer, format, layout, key);

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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(Error::new(ErrorKind::Io, error.to_string())
                    .with_origin(Origin::File(path.to_owned())))
            }
        };

        let named = path.display().to_string();
        let plaintext = crate::decrypt::decrypt(&ciphertext, &named)?;
        let parsed = crate::document::parse_with(layout.reader(), plaintext.text(), format)
            .map_err(|error| error.with_origin(Origin::File(path.to_owned())))?;

        let (section, siblings) = section_of(table_of(parsed), layout, key)?;
        into.document(layer, &Origin::File(path.to_owned()), section, siblings);

        Ok(())
    }
}

/// One listed source's contribution, whatever kind it is.
pub(super) fn collect_source(
    into: &mut crate::resolve::Collected,
    layer: &'static str,
    source: &Source<'_>,
    layout: Layout<'_>,
    key: &str,
) -> Result<(), Error> {
    if let Some(foreign) = source.foreign_config() {
        let values = crate::backend::config_rs::layer(foreign)?;

        if !values.is_empty() {
            into.layer(layer, Origin::Inline, values);
        }

        return Ok(());
    }

    #[cfg(feature = "figment")]
    if let Some(provider) = source.foreign() {
        if let Some(values) = crate::backend::figment::section_of(provider, key)? {
            // What the provider says about itself, rather than `Inline` for
            // everything: a provider that sets `Metadata::from(name, path)`
            // is documented to trace back to that path, and did not.
            into.layer(layer, crate::backend::figment::origin_of(provider), values);
        }

        return Ok(());
    }

    let Some(format) = source.format() else {
        return Ok(());
    };

    match source.path() {
        Some(path) if source.is_encrypted() => {
            collect_encrypted_file(into, layer, Path::new(path), format, layout, key)
        }
        Some(path) => collect_file(into, layer, Path::new(path), format, layout, key),
        None => {
            let parsed = crate::document::parse_with(
                layout.reader(),
                source.inline_text().unwrap_or_default(),
                format,
            )?;

            let (section, siblings) = section_of(table_of(parsed), layout, key)?;
            into.document(layer, &Origin::Inline, section, siblings);

            Ok(())
        }
    }
}

/// One document, narrowed to the section being loaded.
///
/// The same reading [`Parsed`] does, answering with the section's own values
/// instead of filing every section under a name: whole-document layouts take
/// the document, sectioned ones take the subtree, and a top-level key that
/// is not a table is refused either way — a malformed sibling section is a
/// malformed file.
///
/// `None` when the document says nothing about this section, which is the
/// ordinary case for a file that holds somebody else's.
pub(super) fn section_of(
    document: Table,
    layout: Layout<'_>,
    key: &str,
) -> Result<(Option<Table>, std::collections::BTreeMap<String, Table>), Error> {
    if layout.whole.is_some() {
        let mut values = document;
        values.remove(SCHEMA_KEY);

        return Ok((Some(values), std::collections::BTreeMap::new()));
    }

    let mut section = None;
    // The document's other sections travel with it: a cross-section alias
    // reads the section a key used to live in, and every source of this load
    // was parsed whole anyway — so the values are already here, and finding
    // them costs no second read.
    let mut siblings = std::collections::BTreeMap::new();

    for (name, value) in document {
        if name == SCHEMA_KEY {
            continue;
        }

        let crate::Value::Table(table) = value else {
            return Err(Error::new(
                ErrorKind::Parse,
                format!(
                    "top-level key `{name}` is not a table; every top-level key \
                     in a config file is a section, so a value there must be a \
                     table (`{SCHEMA_KEY}` is the one exception). If this file \
                     is not sectioned — if the whole of it is one \
                     configuration — read it with `.whole_document()`"
                ),
            ));
        };

        if name == key {
            section = Some(table);
        } else {
            siblings.insert(name, table);
        }
    }

    Ok((section, siblings))
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

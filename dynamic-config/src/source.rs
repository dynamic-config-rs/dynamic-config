//! What to load, and from where.

/// Separator that introduces one level of nesting in a variable name.
///
/// A single underscore cannot mean both "word break" and "nesting", so nesting
/// gets the doubled form: `APP_DB_POOL__MAX_SIZE` is `pool.max_size`, while
/// `APP_DB_MAX_SIZE` is the single field `max_size`.
pub const DEFAULT_NEST: &str = "__";

/// A configuration file format.
///
/// Every variant exists regardless of which features are enabled; parsing one
/// whose feature is off is a runtime [`ErrorKind::Backend`](crate::ErrorKind)
/// naming the feature to enable. The check is at load time by design: since
/// 0.2 the macro takes no source arguments, so the extension is first seen
/// when the file is.
/// `#[non_exhaustive]` as of 0.7: a format list grows, and a match over
/// one needs a `_` arm — the one break that makes every later format
/// additive.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// JSON, via the `json` feature.
    Json,
    /// TOML, via the `toml` feature.
    Toml,
    /// YAML, via the `yaml` feature.
    Yaml,
    /// INI, via the `ini` feature. `[a.b]` nests; the dialect is spelled
    /// out in the book's Formats chapter.
    Ini,
    /// Java-style `.properties`, via the `properties` feature. UTF-8,
    /// dotted keys nest, collisions are errors.
    Properties,
}

impl Format {
    /// The format a path's extension names.
    ///
    /// A `.age` suffix is looked *through*: `secrets.json.age` is JSON that
    /// happens to be encrypted, so the format is the one under the suffix.
    ///
    /// # Errors
    ///
    /// If the name resolves to no supported format.
    pub fn from_path(path: &std::path::Path) -> Result<Self, crate::Error> {
        let named = path.to_str().unwrap_or_default();
        let inner = inner_name(named).map_or(named, |(inner, _)| inner);

        std::path::Path::new(inner)
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(Self::from_extension)
            .ok_or_else(|| crate::Error::unsupported(path))
    }

    /// The cargo feature that enables this format.
    #[must_use]
    pub fn feature(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Ini => "ini",
            Self::Properties => "properties",
        }
    }

    /// Infers a format from a file extension, case-insensitively.
    ///
    /// Returns `None` for an unknown or absent extension. The macro performs
    /// the same inference at compile time.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "toml" => Some(Self::Toml),
            "yaml" | "yml" => Some(Self::Yaml),
            "ini" => Some(Self::Ini),
            "properties" => Some(Self::Properties),
            _ => None,
        }
    }

    /// Infers a format from a store key's extension: `app/config.json` is
    /// JSON.
    ///
    /// What every remote store crate does with the key it was given, written
    /// once. Returns `None` when the key has no extension or an unknown one —
    /// the store's `with_format` exists for exactly that case.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        std::path::Path::new(key)
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(Self::from_extension)
    }
}

/// No `Debug` or `PartialEq`: a `&dyn Provider` is neither, and equality of
/// sources was never something a caller could rely on. `Source` renders itself
/// by hand instead — see its `Debug` impl.
#[derive(Clone, Copy)]
enum Kind<'a> {
    File(&'a str),
    /// A file whose bytes are ciphertext. `format` describes the plaintext.
    Encrypted(&'a str),
    Inline(&'a str),
    /// Somebody else's figment provider, merged in place.
    #[cfg(feature = "figment")]
    Provider(&'a (dyn figment::Provider + Send + Sync)),
}

/// One layer of configuration.
///
/// Constructors are `const`, so a `&'static [Source<'static>]` can live in a
/// `static` — which is how the macro emits it. The lifetime is there for
/// everything else: configuration assembled at runtime, fetched over the
/// network, or read from a pipe borrows just as happily.
#[derive(Clone, Copy)]
pub struct Source<'a> {
    kind: Kind<'a>,
    format: Format,
}

impl<'a> Source<'a> {
    /// A file on disk. A file that does not exist is skipped, not an error —
    /// listing an optional `secrets.toml` is the whole point of layering.
    #[must_use]
    pub const fn file(path: &'a str, format: Format) -> Self {
        Self {
            kind: Kind::File(path),
            format,
        }
    }

    /// Configuration already in memory: a compiled-in default, a fixture, or
    /// something just read off a socket.
    ///
    /// figment parses from a string, so anything implementing [`Read`] arrives
    /// here through [`std::io::read_to_string`] rather than through a variant
    /// of its own — a reader source would only hide that one line.
    ///
    /// ```
    /// # #[cfg(feature = "json")] {
    /// use dynamic_config::{load, Format, LoadSpec, Source};
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct Db { host: String }
    ///
    /// let mut pipe = std::io::Cursor::new(r#"{"db": {"host": "localhost"}}"#);
    /// let text = std::io::read_to_string(&mut pipe).unwrap();
    ///
    /// let sources = [Source::inline(&text, Format::Json)];
    /// let db: Db = load(&LoadSpec::new("db", &sources)).unwrap();
    ///
    /// assert_eq!(db.host, "localhost");
    /// # }
    /// ```
    ///
    /// [`Read`]: std::io::Read
    #[must_use]
    pub const fn inline(text: &'a str, format: Format) -> Self {
        Self {
            kind: Kind::Inline(text),
            format,
        }
    }

    /// Configuration from a figment provider of your own.
    ///
    /// The three built-in kinds — a file, an encrypted file, inline text — are
    /// the ones this crate can describe. A provider is anything figment can
    /// read: `Serialized::defaults(T)`, an `Env` with a filter this crate does
    /// not model, a provider you wrote, one from another crate.
    ///
    /// ```
    /// # #[cfg(all(feature = "figment", feature = "json"))] {
    /// use dynamic_config::{load, LoadSpec, Source};
    /// use figment::providers::{Format as _, Json};
    /// # use serde::Deserialize;
    /// # #[derive(Deserialize)] struct Db { host: String }
    ///
    /// // `.nested()` because this crate reads a top-level key as a section.
    /// let provider = Json::string(r#"{"db": {"host": "localhost"}}"#).nested();
    /// let sources = [Source::provider(&provider)];
    ///
    /// let db: Db = load(&LoadSpec::new("db", &sources)).unwrap();
    /// assert_eq!(db.host, "localhost");
    /// # }
    /// ```
    ///
    /// # Two things it is on you to get right
    ///
    /// **Sections.** Every other source here goes through this crate's own
    /// mapping of top-level keys to sections. A provider does not: what it
    /// yields is merged as figment sees it, so it has to produce the section as
    /// a profile — `.nested()` on a figment `Data` provider does exactly that.
    /// The loader namespaces section profiles internally (so a section named
    /// `global` cannot collide with figment's reserved profiles); the prefix
    /// is applied *for* the provider on the way in, and `default` / `global`
    /// pass through untouched — for a provider author they are figment's own
    /// vocabulary, deliberately reachable through this one door.
    ///
    /// **Provenance comes from the metadata's *source*, not its name.**
    /// `Metadata::named("INI file")` alone leaves every value it supplies
    /// answering [`Origin::Unknown`](crate::Origin::Unknown): the name
    /// reaches error messages, but
    /// [`source_of`](crate::source_of) reads the source. Set both —
    /// `Metadata::from("INI file", path)` — and a value traces back to the
    /// file that holds it, exactly as one from `.file(..)` does. A provider
    /// that describes itself badly produces a diagnostic that describes it
    /// badly; one that describes itself not at all produces none.
    ///
    /// The `Send + Sync` bound is not decoration: a `LoadSpec` is moved to
    /// another thread by `load_async` and by the file watcher, so a provider
    /// that cannot cross one would take those with it. Every provider figment
    /// ships already satisfies it.
    #[cfg(feature = "figment")]
    #[cfg_attr(docsrs, doc(cfg(feature = "figment")))]
    #[must_use]
    pub const fn provider(provider: &'a (dyn figment::Provider + Send + Sync)) -> Self {
        Self {
            kind: Kind::Provider(provider),
            // A provider parses nothing: it hands over values that are already
            // figment's. `format()` says so by answering `None`.
            format: Format::Json,
        }
    }

    /// This source's format, for the kinds that parse text.
    ///
    /// `None` for a [`provider`](Self::provider), which hands over values that
    /// are already figment's and never sees a byte of text.
    #[must_use]
    pub const fn format(&self) -> Option<Format> {
        match self.kind {
            #[cfg(feature = "figment")]
            Kind::Provider(_) => None,
            _ => Some(self.format),
        }
    }

    /// Configuration in a file that is encrypted on disk.
    ///
    /// `format` is what the *plaintext* is, so `secrets.json.age` is
    /// [`Format::Json`]. Reading it needs a decryptor installed with
    /// [`set_decryptor`](crate::set_decryptor).
    ///
    /// In every other respect it is a file: same precedence, same profile
    /// variants, watched the same way, skipped if it is not there.
    #[must_use]
    pub const fn encrypted(path: &'a str, format: Format) -> Self {
        Self {
            kind: Kind::Encrypted(path),
            format,
        }
    }

    /// The file path, if this source is a file — encrypted or not.
    ///
    /// Encrypted files are included because everything that asks this question
    /// — profile variants, the directories to watch — wants the same answer for
    /// both.
    #[must_use]
    pub const fn path(&self) -> Option<&'a str> {
        match self.kind {
            Kind::File(path) | Kind::Encrypted(path) => Some(path),
            _ => None,
        }
    }

    /// Whether this source has to be decrypted before it can be parsed.
    #[must_use]
    pub const fn is_encrypted(&self) -> bool {
        matches!(self.kind, Kind::Encrypted(_))
    }

    /// The embedded text, if this source is inline.
    pub(crate) fn inline_text(&self) -> Option<&'a str> {
        match self.kind {
            Kind::Inline(text) => Some(text),
            _ => None,
        }
    }

    /// The foreign provider, if this source is one.
    #[cfg(feature = "figment")]
    pub(crate) fn foreign(&self) -> Option<&'a (dyn figment::Provider + Send + Sync)> {
        match self.kind {
            Kind::Provider(provider) => Some(provider),
            _ => None,
        }
    }
}

impl std::fmt::Debug for Source<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            Kind::File(path) => f.debug_tuple("File").field(&path).finish(),
            Kind::Encrypted(path) => f.debug_tuple("Encrypted").field(&path).finish(),
            // Never the text: an inline source is as likely to hold secrets as
            // a file is, and this is the type that would print them.
            Kind::Inline(text) => f
                .debug_struct("Inline")
                .field("bytes", &text.len())
                .field("format", &self.format)
                .finish(),
            #[cfg(feature = "figment")]
            Kind::Provider(provider) => f
                .debug_tuple("Provider")
                .field(&provider.metadata().name)
                .finish(),
        }
    }
}

/// The suffix that marks a config file as encrypted.
///
/// Lives here rather than with the decryption itself because the *naming* rules
/// — which files are encrypted, what a profile variant of one is called — are
/// needed whether or not this build can decrypt anything.
pub(crate) const ENCRYPTED_SUFFIX: &str = "age";

/// Splits `secrets.json.age` into `("secrets.json", "json")`.
///
/// `None` when the name does not end in the suffix, or has no extension under
/// it to name a format.
pub(crate) fn inner_name(path: &str) -> Option<(&str, &str)> {
    let stripped = path.strip_suffix(ENCRYPTED_SUFFIX)?.strip_suffix('.')?;
    let extension = std::path::Path::new(stripped)
        .extension()
        .and_then(|extension| extension.to_str())?;

    Some((stripped, extension))
}

/// Everything the loader needs: which layers, which section, which env prefix.
///
/// Prefer [`new`](Self::new) and the `with_*` methods over a struct literal, so
/// that a later release can add a knob without breaking every call site.
#[derive(Clone, Copy)]
pub struct LoadSpec<'a> {
    /// The configuration section this maps to, e.g. `"db"`.
    pub key: &'a str,
    /// Layers, merged left to right. Later sources win.
    pub sources: &'a [Source<'a>],
    /// Environment variable prefix, e.g. `"APP_"`. Combined with `key`.
    /// `None` ignores the environment entirely.
    pub env_prefix: Option<&'a str>,
    /// Where to look for configuration files by name, if anywhere.
    ///
    /// Discovered files are merged *before* [`sources`](Self::sources), so an
    /// explicitly listed file still has the last word.
    pub search: Option<crate::Search<'a>>,
    /// Environment variable naming the active profile, e.g. `"APP_ENV"`.
    ///
    /// When it is set to `production`, every file gains a sibling layer:
    /// `config.toml` is followed by `config.production.toml`, discovered or
    /// listed alike. A variant that does not exist is skipped like any other
    /// missing file.
    pub profile_env: Option<&'a str>,
    /// Values below the files: consulted only when nothing else supplies a key.
    pub defaults: Option<&'a crate::Layer>,
    /// A document fetched from a remote store: above the files, below the
    /// environment.
    pub remote: Option<&'a crate::Remote>,
    /// A directory of single-value files — one file per key, the filename is
    /// the key, the contents are the value.
    ///
    /// How Docker and Kubernetes mount secrets. Nesting is spelled in the
    /// filename with [`nest`](Self::nest), so one setting governs this layer
    /// and the environment alike; a directory that is not there is skipped
    /// like a missing file.
    pub secrets_dir: Option<&'a str>,
    /// `.env` files, read as the environment layer rather than as documents.
    ///
    /// Merged in order, just *below* the real environment: a variable somebody
    /// exported for this run should beat a file in the repository.
    pub env_files: &'a [&'a str],
    /// Old key paths that still resolve, filling a gap rather than overriding.
    pub aliases: Option<&'a crate::Aliases>,
    /// Fields bound to environment variables by name: just above the prefixed
    /// environment layer, because a binding is the more specific statement.
    pub env_bindings: Option<&'a crate::EnvBindings>,
    /// Values from the command line: above the environment, below overrides.
    pub flags: Option<&'a crate::Layer>,
    /// Values above everything, including the environment.
    pub overrides: Option<&'a crate::Layer>,
    /// Separator that introduces nesting in an environment variable name.
    ///
    /// Defaults to `"__"`, so `APP_DB_POOL__MAX_SIZE` is `pool.max_size`. A
    /// single separator cannot mean both "word break" and "nesting", so
    /// whatever this is set to, it has to be something a field name will not
    /// contain.
    pub nest: &'a str,
    /// Whether `FOO=` counts as set-to-empty.
    ///
    /// Defaults to `false`, which treats it as unset. An unset value rendered
    /// into a deployment template leaves exactly `FOO=`, and letting that blank
    /// out a perfectly good configured value is a bad afternoon. Turn it on
    /// when empty really is a value you need to be able to send.
    pub allow_empty_env: bool,
    /// Rejects environment values from the yes/no/on/off family instead of
    /// letting them arrive as strings where a boolean was meant.
    pub strict_env: bool,
    /// Whether a symlink in [`secrets_dir`](Self::secrets_dir) may resolve
    /// outside that directory.
    ///
    /// `false` — the default — refuses the escape with an error naming the
    /// entry: a directory of mounted credentials that silently reads
    /// `/etc/shadow` through a planted link is the vulnerability shape
    /// Pydantic Settings shipped a CVE for in 2026. Links *inside* the
    /// directory keep working — Kubernetes mounts every key as a symlink
    /// through `..data`, and those targets live under the same mount.
    /// `true` restores the old follow-anything behaviour for the rare
    /// deliberate cross-mount layout.
    pub allow_external_symlinks: bool,
    /// Whether the documents this reads carry a section header at all.
    ///
    /// `false` — the default — means every top-level key in a document is a
    /// section, which is what lets one file serve several configuration
    /// types and what [`key`](Self::key) selects out of it.
    ///
    /// `true` means the document *is* this section's values —
    /// `{"host": "0.0.0.0", "port": 8000}`, with no `server` above it. The
    /// key still names the load: the environment prefix, the cache entry
    /// and what a diagnostic calls this configuration are all still built
    /// from it. It simply stops being looked for inside the document. See
    /// [`with_whole_document`](Self::with_whole_document).
    pub whole_document: bool,
}

impl std::fmt::Debug for LoadSpec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadSpec")
            .field("key", &self.key)
            .field("sources", &self.sources)
            .field("search", &self.search)
            .field("profile_env", &self.profile_env)
            .field("env_prefix", &self.env_prefix)
            .field("defaults", &self.defaults.is_some())
            .field("remote", &self.remote.is_some())
            .field("secrets_dir", &self.secrets_dir)
            .field("flags", &self.flags.is_some())
            .field("overrides", &self.overrides.is_some())
            .field("nest", &self.nest)
            .field("allow_empty_env", &self.allow_empty_env)
            .field("strict_env", &self.strict_env)
            .field("whole_document", &self.whole_document)
            .finish()
    }
}

impl<'a> LoadSpec<'a> {
    /// A spec that reads `sources` and selects `key`, ignoring the environment.
    #[must_use]
    pub const fn new(key: &'a str, sources: &'a [Source<'a>]) -> Self {
        Self {
            key,
            sources,
            search: None,
            profile_env: None,
            env_prefix: None,
            defaults: None,
            remote: None,
            secrets_dir: None,
            env_files: &[],
            aliases: None,
            env_bindings: None,
            flags: None,
            overrides: None,
            nest: DEFAULT_NEST,
            allow_empty_env: false,
            strict_env: false,
            allow_external_symlinks: false,
            whole_document: false,
        }
    }

    /// Looks for `{name}.{ext}` in each of `paths`, in order.
    ///
    /// Every directory that has a match contributes one file, so the search
    /// order *is* the layering order. Discovered files sit below the explicit
    /// [`sources`](Self::sources).
    #[must_use]
    pub const fn with_search(mut self, name: &'a str, paths: &'a [&'a str]) -> Self {
        self.search = Some(crate::Search::new(name, paths));
        self
    }

    /// Layers a per-profile sibling over every file.
    ///
    /// `variable` names the environment variable holding the profile, so the
    /// profile itself is resolved at load time rather than baked in.
    #[must_use]
    pub const fn with_profile_env(mut self, variable: &'a str) -> Self {
        self.profile_env = Some(variable);
        self
    }

    /// Values consulted only when no file and no variable supplies a key.
    #[must_use]
    pub const fn with_defaults(mut self, layer: &'a crate::Layer) -> Self {
        self.defaults = Some(layer);
        self
    }

    /// `.env` files, merged just below the real environment.
    ///
    /// Needs the `dotenv` feature; without it, a non-empty list is an error at
    /// load time naming the feature rather than a list silently ignored.
    #[must_use]
    pub const fn with_env_files(mut self, files: &'a [&'a str]) -> Self {
        self.env_files = files;
        self
    }

    /// Old key paths that still resolve.
    #[must_use]
    pub const fn with_aliases(mut self, aliases: &'a crate::Aliases) -> Self {
        self.aliases = Some(aliases);
        self
    }

    /// Fields bound to environment variables by name.
    #[must_use]
    pub const fn with_env_bindings(mut self, bindings: &'a crate::EnvBindings) -> Self {
        self.env_bindings = Some(bindings);
        self
    }

    /// A remote store's document, layered over the files.
    #[must_use]
    pub const fn with_remote(mut self, remote: &'a crate::Remote) -> Self {
        self.remote = Some(remote);
        self
    }

    /// A directory of single-value files, layered just below the `.env` files
    /// and the environment.
    ///
    /// One directory level: every regular file in it is one key, named by the
    /// file and valued by its contents with a single trailing newline
    /// removed. Subdirectories are not descended into — nesting is spelled in
    /// the filename with [`with_nest`](Self::with_nest), which is what a
    /// Kubernetes mount produces anyway.
    #[must_use]
    pub const fn with_secrets_dir(mut self, path: &'a str) -> Self {
        self.secrets_dir = Some(path);
        self
    }

    /// Sets [`allow_external_symlinks`](Self::allow_external_symlinks).
    #[must_use]
    pub const fn with_allow_external_symlinks(mut self, allow: bool) -> Self {
        self.allow_external_symlinks = allow;
        self
    }

    /// Values from the command line, layered over the environment.
    #[must_use]
    pub const fn with_flags(mut self, layer: &'a crate::Layer) -> Self {
        self.flags = Some(layer);
        self
    }

    /// Values that win over the files, the environment and the flags alike.
    #[must_use]
    pub const fn with_overrides(mut self, layer: &'a crate::Layer) -> Self {
        self.overrides = Some(layer);
        self
    }

    /// Layers environment variables named `{prefix}{KEY}_*` over the files.
    #[must_use]
    pub const fn with_env(mut self, prefix: &'a str) -> Self {
        self.env_prefix = Some(prefix);
        self
    }

    /// Uses `separator` instead of `__` to introduce nesting.
    #[must_use]
    pub const fn with_nest(mut self, separator: &'a str) -> Self {
        self.nest = separator;
        self
    }

    /// Treats `FOO=` as set-to-empty rather than unset.
    #[must_use]
    pub const fn with_empty_env(mut self, allow: bool) -> Self {
        self.allow_empty_env = allow;
        self
    }

    /// Rejects ambiguous environment spellings instead of guessing.
    ///
    /// `APP_DB_TLS=off` reads like a boolean and arrives as the string
    /// `"off"` — silently correct into a `String` field, silently wrong
    /// everywhere else. Strict mode makes the yes/no/on/off family (and
    /// `null`/`nil`/`none`) an error naming the variable; write `true`,
    /// `false`, or the value you actually mean.
    #[must_use]
    pub const fn with_strict_env(mut self, strict: bool) -> Self {
        self.strict_env = strict;
        self
    }

    /// Reads each document as this section's values, with no section header.
    ///
    /// The default layout is one file, several sections: every top-level key
    /// names one, and [`key`](Self::key) says which is yours. That is what
    /// lets a `config.toml` hold `[db]` and `[server]` for two configuration
    /// types that know nothing about each other.
    ///
    /// A file that is *only* this configuration has no use for the header,
    /// and a file this crate did not write may not have one to begin with —
    /// a container image's `{"host": "0.0.0.0", "port": 8000}`, a chart's
    /// rendered values, a file some other tool owns. This says so.
    ///
    /// Everything else is unchanged, and that is the point: the environment
    /// prefix is still `{prefix}{KEY}_`, profile variants
    /// (`config.production.toml`) still layer on top, defaults, flags,
    /// overrides, aliases, the secrets directory, the cache and every
    /// diagnostic all behave exactly as they do for a sectioned load. Only
    /// where a document's values are found changes.
    ///
    /// It applies to **every** document this spec reads — listed files,
    /// discovered files, inline text and the remote store's document —
    /// because a load whose sources disagreed about their own shape would be
    /// a load nobody could reason about.
    #[must_use]
    pub const fn with_whole_document(mut self, whole: bool) -> Self {
        self.whole_document = whole;
        self
    }

    /// The environment variable that names the profile, if configured.
    pub(crate) fn profile_variable(&self) -> Option<&'a str> {
        self.profile_env
    }

    /// The active profile, if one is named and set to something.
    pub(crate) fn profile(&self) -> Option<String> {
        self.profile_env
            .and_then(|variable| std::env::var(variable).ok())
            .map(|profile| profile.trim().to_owned())
            .filter(|profile| !profile.is_empty())
    }

    /// The full environment prefix for this section: `"APP_"` + `"db"` → `"APP_DB_"`.
    ///
    /// An empty key contributes nothing rather than an extra underscore: a
    /// whole-document load may have no name to give the section, and
    /// `APP__HOST` is a variable nobody would guess they had to set.
    pub(crate) fn full_env_prefix(&self) -> Option<String> {
        self.env_prefix.map(|prefix| {
            if self.key.is_empty() {
                prefix.to_owned()
            } else {
                format!("{prefix}{}_", self.key.to_ascii_uppercase())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_formats_case_insensitively() {
        assert_eq!(Format::from_extension("JSON"), Some(Format::Json));
        assert_eq!(Format::from_extension("yml"), Some(Format::Yaml));
        assert_eq!(Format::from_extension("yaml"), Some(Format::Yaml));
        assert_eq!(Format::from_extension("ini"), Some(Format::Ini));
        assert_eq!(
            Format::from_extension("properties"),
            Some(Format::Properties)
        );
        assert_eq!(Format::from_extension("conf"), None);
    }

    #[test]
    fn the_env_prefix_combines_the_caller_prefix_with_the_key() {
        let spec = LoadSpec::new("db", &[]).with_env("APP_");

        assert_eq!(spec.full_env_prefix().as_deref(), Some("APP_DB_"));
    }

    #[test]
    fn no_env_prefix_means_no_environment() {
        assert_eq!(LoadSpec::new("db", &[]).full_env_prefix(), None);
    }

    #[test]
    fn an_empty_environment_variable_is_unset_by_default() {
        assert!(!LoadSpec::new("db", &[]).allow_empty_env);
        assert!(
            LoadSpec::new("db", &[])
                .with_empty_env(true)
                .allow_empty_env
        );
    }

    #[test]
    fn only_file_sources_expose_a_path() {
        assert_eq!(Source::file("a.json", Format::Json).path(), Some("a.json"));
        assert_eq!(Source::inline("{}", Format::Json).path(), None);
    }

    #[test]
    fn a_source_can_borrow_from_a_runtime_string() {
        let text = String::from("{}");
        let source = Source::inline(&text, Format::Json);

        assert_eq!(source.path(), None);
    }
}

//! Reading a document: the seam between text and this crate's tree.
//!
//! A reader answers one question — *what does this text say, in this
//! format?* — and answers it with a [`Value`]. Everything above it works
//! on that tree and never on a parser, which is what lets the parser be a
//! choice.
//!
//! ```text
//! "[db]\nport = 5432\n"  +  Format::Toml   ──▶  {db: {port: 5432}}
//! ```
//!
//! Three ship, and the reason to choose between them is not taste:
//!
//! | reader | feature | parses | notes |
//! |---|---|---|---|
//! | [`native()`] — **the default** | — | JSON, TOML, YAML, INI, `.properties` | the only `.properties` parser anywhere |
//! | [`config_rs()`] | — | JSON, TOML, YAML, INI, RON, JSON5 | YAML through the maintained `yaml-rust2` |
//! | [`figment()`] | `figment` | JSON, TOML, YAML | |
//!
//! The column is what each one **parses**, not what a load that chose it
//! can read: a format the chosen reader has no parser for is handed to one
//! that has — so choosing `config_rs()` for its YAML does not cost you the
//! `.properties` file beside it.
//!
//! **Unlike the [engines](crate::engine), readers are not interchangeable
//! down to the corner.** A fold is one rule with an implementation on
//! each side; a parser is a *dialect*, and two YAML libraries disagree about
//! things no specification settles. What the tests hold is the part a
//! deployment depends on — the shapes documents actually take — and the
//! places they diverge are named in the book rather than papered over.
//!
//! Which one runs is the same choice the engine is: `Builder::reader`,
//! [`LoadSpec::with_reader`](crate::LoadSpec::with_reader), or
//! [`set_reader`] once for the process.

use std::fmt;

use crate::error::{Error, ErrorKind};
use crate::source::Format;
use crate::value::Value;

/// Text in, this crate's tree out.
///
/// Implement it to read a format this crate does not ship, or to read one
/// it does with a parser of your own.
///
/// # Errors
///
/// A reader's error must **never carry document content**. The line that
/// failed to parse is, on a bad day, the line holding the password — so a
/// message says where it stopped and why, and never what it found there.
pub trait Reader: fmt::Debug + Send + Sync {
    /// What to call this reader in a diagnostic.
    fn name(&self) -> &str;

    /// Whether this reader can read `format` in this build.
    ///
    /// A reader whose backend was compiled without a format answers
    /// `false` for it, and the load says which reader could have.
    fn reads(&self, format: Format) -> bool;

    /// `text`, as this crate's tree.
    ///
    /// The result is always a [`Value::Table`]: a document is keys.
    ///
    /// # Errors
    ///
    /// If the text is not valid in its format.
    fn parse(&self, text: &str, format: Format) -> Result<Value, Error>;
}

/// This crate's own parsers.
///
/// The only reader that reads `.properties`, and the one whose INI dialect
/// the book documents.
#[must_use]
pub fn native() -> &'static dyn Reader {
    &Native
}

/// The [`config`](https://docs.rs/config) crate's parsers.
///
/// **Not the default** — [`native()`] is, and the module's own
/// documentation says why. This is the engine's opposite: there the
/// backend's fold is what runs unless a load says otherwise, because a
/// fold can be proved interchangeable and a parser is a dialect.
///
/// Reads two formats nothing else here does — RON and JSON5 — and reads
/// YAML through `yaml-rust2`, which is maintained where this crate's own
/// `serde_yaml` is archived.
#[must_use]
pub fn config_rs() -> &'static dyn Reader {
    &ConfigRs
}

/// The [`figment`](https://docs.rs/figment) crate's parsers.
#[cfg(feature = "figment")]
#[cfg_attr(docsrs, doc(cfg(feature = "figment")))]
#[must_use]
pub fn figment() -> &'static dyn Reader {
    &Figment
}

/// Every reader this build ships, this crate's own first.
///
/// **The one list.** The agreement tests walk it, so a reader added here
/// is compared against the others on every corpus without a test being
/// edited.
#[must_use]
pub fn all() -> Vec<&'static dyn Reader> {
    vec![
        native(),
        config_rs(),
        #[cfg(feature = "figment")]
        figment(),
    ]
}

/// The reader a load uses when nothing chose one: this crate's own.
///
/// **Not the backend, unlike the [engine](crate::engine).** The asymmetry
/// is deliberate and it is about what can be proved. A fold is one rule
/// with an implementation on each side, and the tests hold both to it leaf
/// by leaf, so which one runs is not a question about meaning. A parser is
/// a *dialect*: this crate's INI is the one the book specifies, and
/// `.properties` has no parser anywhere else — every reader reads one, and
/// it is this one. Handing those to a different
/// library by default would change what a document means for everyone who
/// upgraded, quietly.
///
/// The backend's parsers are one call away, and worth having — see
/// [`config_rs`].
pub(crate) fn default() -> &'static dyn Reader {
    native()
}

static INSTALLED: std::sync::OnceLock<&'static dyn Reader> = std::sync::OnceLock::new();

/// The installed reader, or the default.
pub(crate) fn installed() -> &'static dyn Reader {
    INSTALLED.get().copied().unwrap_or_else(default)
}

/// Installs `reader` for every load in this process that does not name one.
///
/// Call it before the first `init()`.
///
/// # Errors
///
/// If one is already installed. The rejected reader is returned, so a
/// caller can tell "already set" from "failed".
pub fn set_reader(reader: &'static dyn Reader) -> Result<(), &'static dyn Reader> {
    INSTALLED.set(reader)
}

/// Whether a reader has been installed.
#[must_use]
pub fn has_reader() -> bool {
    INSTALLED.get().is_some()
}

/// The reader that will parse `format`: the chosen one, or the first that
/// can.
///
/// A reader that does not read a format hands it on rather than refusing
/// it, which is what makes a format like RON — parsed by the backend and
/// by nothing here — work without a caller having to install a reader by
/// hand. Deterministic, in [`all`]'s order, and **additive**: the fallback
/// can only fire where the chosen reader would have failed outright, so a
/// load that worked keeps working through exactly the same parser.
pub(crate) fn for_format(
    chosen: &'static dyn Reader,
    format: Format,
) -> Option<&'static dyn Reader> {
    if chosen.reads(format) {
        return Some(chosen);
    }

    all().into_iter().find(|reader| reader.reads(format))
}

/// Nobody in this build reads `format`.
///
/// Names the readers that *could*, because the answer is almost always a
/// feature: this crate's own for `.properties`, the backend's for RON.
pub(crate) fn unread(format: Format) -> Error {
    let readers: Vec<&str> = all()
        .into_iter()
        .filter(|reader| reader.reads(format))
        .map(Reader::name)
        .collect();

    let advice = if readers.is_empty() {
        format!(
            "add features = [\"{}\"] to your dynamic-config dependency",
            format.feature()
        )
    } else {
        // Reachable by calling a reader's `parse` directly, and not by
        // loading: a load hands a format its chosen reader cannot read to
        // one that can. So the advice is about *this call*, not about a
        // build that is missing something.
        format!(
            "the {} reader{} in this build read{} it, and a load hands the \
             format to whichever does — this call named one that does not",
            readers.join(" and the "),
            if readers.len() > 1 { "s" } else { "" },
            if readers.len() > 1 { "" } else { "s" },
        )
    };

    Error::new(
        ErrorKind::Backend,
        format!("nothing here reads {format:?}: {advice}"),
    )
}

// ---------------------------------------------------------------------------
// This crate's own
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Native;

impl Reader for Native {
    fn name(&self) -> &str {
        "native"
    }

    fn reads(&self, format: Format) -> bool {
        match format {
            Format::Json => cfg!(feature = "json"),
            Format::Toml => cfg!(feature = "toml"),
            Format::Yaml => cfg!(feature = "yaml"),
            Format::Ini => cfg!(feature = "ini"),
            Format::Properties => cfg!(feature = "properties"),
            // Neither has a writer here, and this crate's readers are the
            // half that has to round-trip with one.
            Format::Ron | Format::Json5 => false,
        }
    }

    fn parse(&self, text: &str, format: Format) -> Result<Value, Error> {
        crate::document::parse_natively(text, format)
    }
}

// ---------------------------------------------------------------------------
// config-rs
// ---------------------------------------------------------------------------

use crate::backend::config_rs::Reader as ConfigRs;

// ---------------------------------------------------------------------------
// figment
// ---------------------------------------------------------------------------

#[cfg(feature = "figment")]
use crate::backend::figment::Reader as Figment;

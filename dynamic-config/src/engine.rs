//! The resolution engine: the thing that folds the collected layers.
//!
//! A load walks its sources itself — discovery, decryption, sections, the
//! environment and the `.env` files are this crate's, and no engine sees a
//! file. What an engine does is the step after that: take one tree per
//! layer, in precedence order, and answer with the merged tree and which
//! layer won each leaf.
//!
//! ```text
//! defaults   {host: localhost, port: 5432}   tag 0
//! file       {host: db.internal}             tag 1
//! env        {port: 6543}                    tag 2
//!            ──────────── fold ────────────
//! values     {host: db.internal, port: 6543}
//! tags       host → 1, port → 2
//! ```
//!
//! **Every engine here implements the same rule** — tables descend,
//! everything else replaces, arrays included — so which one is installed is
//! not a question about what a configuration means. That is not a hope: the
//! whole composition corpus, a generated corpus of layer stacks, and the
//! corner cases where a backend's own habits could show through all run
//! through every engine, and both the tree and the winner of every leaf are
//! compared.
//!
//! Where a backend disagrees, the adapter is what gives way. One of them
//! reads a top-level key as a *path expression*, which would turn
//! `{"my.module": "debug"}` into a nested table; its adapter hands over
//! stand-in names and puts the document's own back afterwards. Another
//! records provenance per provider and cannot answer for a key with a dot
//! in it; the fold fills that in from the layers it was given.
//!
//! Two ship:
//!
//! | engine | feature | what it is |
//! |---|---|---|
//! | [`config_rs()`] | — | the fold of the `config` crate; the default |
//! | [`figment()`] | `figment` | the fold of the `figment` crate |
//!
//! A third is anything implementing [`Engine`]: the trait deals in this
//! crate's [`Value`] and in opaque tags, so nothing about a backend reaches
//! it. **This crate keeps no fold of its own** — it wrote one, proved the
//! others against it, and then deleted it rather than maintain a second
//! implementation of somebody else's rule.

use std::collections::BTreeMap;
use std::fmt;

use crate::error::Error;
use crate::value::Value;

/// One layer, on its way into an engine.
///
/// The tag is opaque and belongs to the caller: hand it back for every leaf
/// this layer wins, and the caller turns it into the file or variable a
/// person reads. An engine that invents a tag it was not given is an engine
/// reporting a source that does not exist.
#[derive(Debug, Clone, Copy)]
pub struct Layer<'a> {
    /// Which layer this is, as the caller counts them.
    pub tag: usize,
    /// What it has to say: always a [`Value::Table`], because a section is
    /// keys and a layer with nothing to say supplies an empty one.
    pub values: &'a Value,
}

/// What an engine hands back.
#[derive(Debug, Clone, PartialEq)]
pub struct Folded {
    /// The merged tree, always a [`Value::Table`].
    pub values: Value,
    /// The winning layer's tag per dotted leaf path.
    ///
    /// A leaf with no entry reports [`Origin::Unknown`](crate::Origin) —
    /// honest, and better than a guess.
    pub tags: BTreeMap<String, usize>,
}

/// A fold, and where each leaf came from.
///
/// Implement this to resolve with something else entirely — a backend this
/// crate does not ship, or a rule of your own. Two things are asked of an
/// implementation, and the rest is its business:
///
/// - **Precedence is the argument's order.** `layers[0]` is the lowest.
/// - **A tag is reported only for a leaf that layer actually supplied.**
///
/// # Errors
///
/// A fold can fail — a backend may refuse a key shape of its own — and the
/// error reaches the caller as an ordinary load failure. It must not carry
/// a configuration value: an engine that puts one in a message breaks the
/// contract every other part of this crate keeps.
pub trait Engine: fmt::Debug + Send + Sync {
    /// What to call this engine in a diagnostic.
    fn name(&self) -> &str;

    /// Folds `layers`, lowest precedence first.
    ///
    /// # Errors
    ///
    /// If the backend refuses the shape it was handed.
    fn fold(&self, layers: &[Layer<'_>]) -> Result<Folded, Error>;
}

/// The [`config`](https://docs.rs/config) crate's fold.
///
/// The default. `config` carries an origin on every value, so a leaf's
/// winner comes back from the backend rather than from a second walk.
#[must_use]
pub fn config_rs() -> &'static dyn Engine {
    &ConfigRs
}

/// The [`figment`](https://docs.rs/figment) crate's fold.
///
/// figment records metadata per *provider*, so each layer is merged as its
/// own provider and the winner of a leaf is read back from the metadata
/// that survived the merge.
#[cfg(feature = "figment")]
#[cfg_attr(docsrs, doc(cfg(feature = "figment")))]
#[must_use]
pub fn figment() -> &'static dyn Engine {
    &Figment
}

/// Every engine this build ships, lowest-level first.
///
/// **The one list.** The agreement tests walk it rather than naming
/// engines, so an engine added here is compared against the others on every
/// corpus the crate has without a test being edited — which is the point:
/// an engine that nothing compares is an engine nobody has checked.
///
/// The default comes first, and is the one a disagreement is reported
/// against.
#[must_use]
pub fn all() -> Vec<&'static dyn Engine> {
    vec![
        config_rs(),
        #[cfg(feature = "figment")]
        figment(),
    ]
}

/// The engine a load uses when nothing chose one.
///
/// **This crate has no fold of its own.** It had one, as the reference the
/// others were compared against — and carrying a second implementation of
/// a rule somebody else already implements is maintenance with no reader.
/// The comparison it existed for is between the backends; the rule itself
/// is written down in the book and held by the tests either way.
pub(crate) fn default() -> &'static dyn Engine {
    config_rs()
}

/// The installed engine, or the default.
pub(crate) fn installed() -> &'static dyn Engine {
    INSTALLED.get().copied().unwrap_or_else(default)
}

static INSTALLED: std::sync::OnceLock<&'static dyn Engine> = std::sync::OnceLock::new();

/// Installs `engine` for every load in this process that does not name one.
///
/// Call it before the first `init()`. A load that names its own engine —
/// `builder(..).engine(..)` — uses that one whatever is installed here.
///
/// # Errors
///
/// If one is already installed. The rejected engine is returned, so a
/// caller can tell "already set" from "failed".
pub fn set_engine(engine: &'static dyn Engine) -> Result<(), &'static dyn Engine> {
    INSTALLED.set(engine)
}

/// Whether an engine has been installed.
#[must_use]
pub fn has_engine() -> bool {
    INSTALLED.get().is_some()
}

// ---------------------------------------------------------------------------
// config-rs
// ---------------------------------------------------------------------------

use crate::backend::config_rs::Engine as ConfigRs;

// ---------------------------------------------------------------------------
// figment
// ---------------------------------------------------------------------------

#[cfg(feature = "figment")]
use crate::backend::figment::Engine as Figment;

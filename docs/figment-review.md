# Where figment shows through

The 1.0 doctrine calls for a design pass on where the underlying loader's
abstractions leak into this crate's contract, before the API freezes around
them. This is that pass: each place figment is visible from the outside,
what the exposure costs, and what — if anything — changes because of it.
The review happened in 0.3; the changes it decided land in 0.4 and are
filed in [ROADMAP.md](../ROADMAP.md).

The one-line summary: two deliberate exposures stay (the `figment` feature
and the loose-parsing default), one accidental exposure is a bug and gets
fixed (reserved profile names), and one limitation gets engineered around
(prefix-grained env provenance).

## Sections ride on figment profiles — and figment reserves two names

**Finding.** The crate maps a file's top-level keys to sections by turning
each key into a figment *profile* (`loader/sections.rs`, the `Sections`
provider) and selecting the builder's key. figment reserves two profile
names with special semantics, and the mapping passes both straight
through:

- A top-level table named **`global`** overrides the same-named fields of
  **every** section — including values the section supplies itself. A
  shared config file with an innocent `global:` table silently rewrites
  every config type that reads the file.
- A top-level table named **`default`** gap-fills every section: any field
  a section does not set is quietly taken from it.

Both are invisible to the crate's own diagnostics: `check` reports no
unknown key, and `source_of` names the file — not the table the value
actually came from. Verified empirically; `app.host` set to `"from-app"`
resolves to `"from-global"` when a `global` table also names `host`.

**Assessment.** This is not a documented feature and not a defensible
accident — it is figment's profile inheritance leaking through a mapping
that was supposed to be a private implementation detail. Nothing in the
book, the error messages, or the type system hints that two ordinary
words are load-bearing.

**Decision — fix in 0.4.** Escape the profile namespace: map every
top-level key `k` to a prefixed profile (`section:k`) and select the
prefixed key, so `global` and `default` become ordinary section names and
the inheritance machinery has nothing to grab. Every layer that files
under "the same profile the files use" — the environment, the runtime
layers, the remote store, the cache — moves with it, which is one constant
in `loader/`. The foreign-provider contract (below) is the one place the
prefix is visible, and the fix must either remap a provider's profile
names in the `Foreign` wrapper or redefine the documented contract; that
trade-off is settled at implementation time, with tests pinning both the
`global` and the `default` behaviour either way.

## Profiles are sibling files, not figment profiles

**Finding.** What this crate calls a profile — `APP_ENV=production`
overlaying `config.production.toml` — is deliberately *not* figment's
profile machinery: it is file-name interpolation, validated against path
traversal (`validated_profile`), with ordinary merge semantics. No figment
concept reaches the user here.

**Decision — keep.** The sibling-file model is simpler to explain than
profile inheritance, visibly diffable (two files on disk), and already
guarded. No change; this entry records that the review looked.

## The `figment` feature is a semver coupling, on purpose

**Finding.** With the `figment` feature on, `pub use figment` and
`Source::provider(&dyn figment::Provider)` make figment's own API part of
this crate's public surface. A figment major release then forces either a
major release here or a compatibility shim.

**Decision — keep, and say so.** The feature exists precisely so that the
long tail of sources this crate will never ship (a database, a Vault
transit mount, someone's in-house format) stays reachable without forking.
The honest framing is a stability-tier note: the `figment` feature is
tier-coupled to figment's semver, and 1.0 of this crate does not extend
its compatibility promise across that boundary. The stability-tiers
chapter gains that sentence in 0.4.

The documented sharp edge stays documented: a foreign provider bypasses
the section mapping and must produce its section as a profile
(`Source::provider`'s docs already say this). It inherits the
reserved-name interaction above until the 0.4 fix settles the namespace.

## Environment provenance is prefix-grained

**Finding.** figment attaches metadata per provider, and the prefixed
environment layer is one provider — so a value or error traced to it says
`APP_DB_*`, not `APP_DB_POOL__MAX_SIZE`. The two per-variable exceptions
already exist because they are one provider per variable: `bind_env`
bindings, and `.env` files (named per file). The prefix layer is the only
coarse one.

**Assessment.** "Which variable do I fix" is the whole point of
provenance, and the crate already holds every ingredient needed to answer
it: the prefix, the section key, the nesting separator, and the key path
the diagnostic is about. The variable name is mechanically derivable.

**Decision — derive it, in 0.4.** When an origin resolves to the prefixed
environment layer, reconstruct the variable name from
`prefix + section + separator + path` and report that instead of the
prefix wildcard. The reconstruction is a naming convention, not a
measurement — if a future figment changes the convention, the loader
contract test (`tests/loader.rs`) is where the drift shows up, so the
derived name is pinned there alongside the existing metadata-name
assertions.

## Reviewed and already settled

Places the review looked and found the line already held, recorded here so
the next pass does not re-litigate them:

- **Loose value parsing** (`APP_APP_TLS=off` arriving as a string) is
  figment's default and stays the default; `strict_env()` is the opt-out,
  documented in the builder tour. A breaking flip would punish every
  working deployment to protect hypothetical ones.
- **Merge semantics** — later wins, tables merge key by key, arrays are
  replaced whole — are figment's, adopted as this crate's documented
  contract in the sources-and-precedence chapter. Adopted is not leaked.
- **Error conversion** (`loader/origin.rs`) already translates figment's
  errors, drops the offending value (the secrets line), and recognises
  each layer's metadata name; the brittleness of name-matching is pinned
  by `tests/loader.rs` rather than left to bug reports.
- **serde_yaml's deprecation** arrives through figment and is tracked in
  the roadmap; a local switch buys nothing until figment moves.

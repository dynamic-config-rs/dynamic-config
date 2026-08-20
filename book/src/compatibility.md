# The Compatibility Contract

What depending on this project means, written down before 1.0 so 1.0
can only confirm it. Every repository's README links here; a release
that would break a line below is a release that does not ship.

## The guarantees

1. **Minor upgrades are source-compatible.** Code that compiles
   against `1.x` compiles against `1.(x+1)`. Pre-1.0, the same promise
   holds within a 0.y line: `0.7.z` never breaks a `0.7` user.
2. **Precedence never silently changes.** The layer order — defaults,
   discovered files, named files, remote, secrets directory, `.env`,
   environment, bindings, `--set`, overrides — is part of the API, and
   [How resolution works](how-resolution-works.md) is the full order
   with the argument for each position. Reordering it is a major
   version, loudly, never a side effect.
3. **Reload-failure semantics never silently change.** A refused
   document leaves the previous snapshot serving — last-known-good is
   a contract, not an implementation detail. The same holds for its
   corollaries: a failed reload bumps no generation, and recovery
   resumes delivery without a restart.
4. **`Origin` and provenance are stable.** `source_of` and the explain
   surface keep naming the same winner for the same stack; provenance
   output only grows fields, never renames or removes them. The origin
   of a leaf is [recorded as the fold runs](how-resolution-works.md),
   so it says which layer won rather than what happened to supply it.
5. **Features are additive-only.** Enabling a cargo feature never
   changes the behaviour of code that compiled without it; no feature
   is ever load-bearing for another's semantics.
6. **Secrets stay out of diagnostics.** No configuration value —
   secret-marked or not — appears in errors, logs, diffs, reports or
   telemetry labels. Pinned by tests and a fuzz target, promised here.
7. **MSRV raises are announced events.** The floor moves in a release
   whose changelog says so in its first lines, never as a lockfile
   accident. The current floor is **Rust 1.88, org-wide** (the Loco
   adapter follows Loco at 1.94); older toolchains resolve older
   published versions via cargo's MSRV-aware resolver and are
   explicitly unsupported.

## Supported versions

Security fixes land on the **latest patch of each currently published
line** — nothing older, no backports before 1.0. When a release ships,
every prior patch of its line is end-of-life the same day. After 1.0,
the current and previous minor lines are supported; that wider promise
starts at 1.0 and not before.

## The one round of exceptions

The stabilisation round that introduced this contract shipped two
knowing breaks as patches, both security work, both called out in
their changelogs: the MSRV raise to 1.88 (unlocking real fixes for
`time`, `serde_with` and the AWS SDK advisories that ignore ledgers
had been carrying), and the secrets-directory symlink containment
default (a symlink escaping the secrets directory is now refused;
`allow_external_symlinks(true)` restores the old behaviour where a
cross-mount layout is genuinely intended).

## Scope

Not a semver tutorial and not a roadmap. The [stability
tiers page](stability-tiers.md) says where the surface is frozen; the
organisation's PATH-TO-1.0 says what evidence 1.0 still wants. This
page is the promise in between: the parts of behaviour you may build
on today and find unchanged tomorrow.

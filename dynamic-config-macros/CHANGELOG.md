# Changelog

All notable changes to `dynamic-config-macros` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Before 1.0, a breaking change bumps the **minor** version and anything else
bumps the patch. A change to the minimum supported Rust version is breaking.

<!-- Keep this template. Add entries under `Unreleased` as you go, and move
     the whole block under a new version heading at release time.
     (Spelled `_Unreleased_` here so cargo-release's `exactly = 1` search
     for the real heading matches only the real heading.)

## [_Unreleased_]

### Added
### Changed
### Deprecated
### Removed
### Fixed
### Security

-->

## [Unreleased]

## [0.6.3] — 2026-08-17

## [0.6.2] — 2026-08-16

## [0.6.1] — 2026-08-14

## [0.6.0] — 2026-08-13

### Fixed

- **A renamed dependency now works.**
  `config = { package = "dynamic-config", version = "0.6" }` is an ordinary
  thing to write, and the expansion hardcoded `::dynamic_config` — a crate
  that consumer's namespace does not have, so every generated item failed to
  resolve. The facade's real name is resolved from the *consumer's*
  manifest with `proc-macro-crate` and threaded through the whole
  expansion.

  `FoundCrate::Itself` is not one case but two, which is what the
  repository's own examples caught: the facade's *library* wants `crate`,
  while an example, a bin or a doctest of that same package links the
  library as an extern crate and wants `::dynamic_config`. Integration
  tests need no special handling — `proc-macro-crate` already tells them
  apart. A consumer with no readable manifest keeps the old hardcoded path,
  so what a user reads is rustc's "unresolved crate" rather than a panic
  from inside a proc macro.

  Doc *links* in the generated documentation still name `::dynamic_config`;
  under a renamed dependency they resolve to nothing and rustdoc warns.
  They are links, not code, and interpolating a path into a doc comment
  costs more than the warning does.

### Changed

- One more dependency, `proc-macro-crate`, and one more parse of the
  consumer's `Cargo.toml` per crate that uses the macro. Measured against
  real 1.71 and 1.74 toolchains before it was added rather than read off a
  manifest: the floor stays at **1.71**.
- The generated install door, `dynamic_config_install`, returns the
  `Arc<Self>` it stored — what `Builder::init_and_current` hands back. It is
  private to the generated impl and named in no documentation, so no user
  code refers to it; the public surface is unchanged.

## [0.5.0] — 2026-08-12

## [0.4.0] — 2026-08-12

## [0.3.0] — 2026-08-11

### Breaking

- The generated `apply_remote` is replaced by `remote_sink()`; see the
  core changelog.

## [0.2.0] — 2026-08-11

### Breaking

- The attribute takes no arguments; the error for any argument is a
  migration map to the builder. Generated code shrinks to the declaration
  surface: storage slots, `builder(key)`, accessors, hooks, remote and
  layer setters, `prepare`, and the redacted `Debug`.
- (superseded in the same release by the builder move) `cache` with no
  `cache_mode` briefly generated the redacted cache; the argument then moved
  to `Builder::cache(path, mode)`, where the mode is always spelled out.

### Added

- Generated `explain(path)` (secrets pre-redacted) and `builder()`; the
  `strict_env` attribute flag.

## [0.1.0] — 2026-08-10

### Breaking

- Generated `start_watch()` requires `Self: 'static` (spelled out) and
  errors on a duplicate start instead of returning an inert handle.

### Added

- Generated `on_reload_scoped` and `set_defaults`.
- Attribute errors are emitted *alongside* the original struct — one macro
  mistake no longer cascades into "cannot find type" at every use site —
  and a non-struct item gets an error that names what the attribute needs.

### Fixed

- The `#[config(secret)]` list is serde-rename-aware (`rename` and
  `rename_all`), so redaction and the JSON schema use the names files
  actually contain.
- Recovery goes through `validate` and seeds the diff baseline.

## [0.0.1] — 2026-08-10

Initial release.

### Added

- The `#[dynamic_config]` attribute: twenty arguments parsed and validated
  with spans, so every mistake is a compile error on the offending token —
  duplicate arguments, `debounce` without `watch`, `paths` without `name`,
  an unknown `cache_mode`, a `.age` suffix with no inner extension.
- Feature-gated generated code routed through redirect macros in the facade
  crate, so a `cfg` is always evaluated where the feature actually exists.
- Compile-fail suites (`trybuild`) pinning the diagnostics, including the
  ones that only exist when a feature is off.

[Unreleased]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.3...HEAD
[0.6.3]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/ctolon/dynamic-config/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/ctolon/dynamic-config/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

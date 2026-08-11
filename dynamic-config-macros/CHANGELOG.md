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

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...HEAD
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

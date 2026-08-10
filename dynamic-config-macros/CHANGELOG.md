# Changelog

All notable changes to `dynamic-config-etcd` are documented here. The format follows
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

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

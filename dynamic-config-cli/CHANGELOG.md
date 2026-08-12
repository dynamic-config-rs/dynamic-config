# Changelog

All notable changes to `dynamic-config-cli` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The crate is Experimental and ships in-repo, not on crates.io; it is
versioned with the workspace all the same, so this file says what changed
when.

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

## [0.4.0] — 2026-08-12

### Breaking

- `explain` redacts by default: every value prints as `***` unless
  `--show-values` is passed. The old `--secret` flag is gone — a published
  diagnostic tool cannot ask its user to already know which paths are
  sensitive, so the safe rendering is the default and seeing values is the
  deliberate act.

### Added

- The crate graduates to crates.io: `cargo install dynamic-config-cli`.
- `completions <shell>` and `man` print shell completions and the manual
  page, both rendered by clap from the same command definition.

## [0.3.0] — 2026-08-11

## [0.2.0] — 2026-08-11

### Added

- The crate: `explain` (every layer's answer for one path, values shown,
  `--secret` to mask) and `diff` (path-only difference between two
  documents) from a shell, the load restated as flags.

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0

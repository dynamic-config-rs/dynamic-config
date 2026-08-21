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

## [0.9.0] — 2026-08-20

## [0.7.1] — 2026-08-19

## [0.7.0] — 2026-08-18

## [0.6.3] — 2026-08-17

## [0.6.2] — 2026-08-16

## [0.6.1] — 2026-08-14

### Fixed

- **`documentation` pointed at another crate.** This crate carried none of
  the workspace's shared metadata, so it inherited the engine's
  `docs.rs/dynamic-config` link — a reader following it from crates.io
  landed on a library rather than on this binary. It now names its own,
  along with the categories and keywords a search for a configuration CLI
  would actually match.

## [0.6.0] — 2026-08-13

## [0.5.0] — 2026-08-12

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

[Unreleased]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.7.1...v0.9.0
[0.7.1]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.3...v0.7.0
[0.6.3]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/ctolon/dynamic-config/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/ctolon/dynamic-config/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0

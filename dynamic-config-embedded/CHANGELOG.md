# Changelog

All notable changes to `dynamic-config-embedded` are documented here. The format follows
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

- `ConfigCell<T>` for `no_std` targets: a snapshot behind a
  `critical-section`, replaced whole, with parsing and validation before
  anything is installed — a bad document leaves the previous configuration
  serving.
- `apply(bytes, Format::Json)` via `serde-json-core`; no allocator anywhere.
- `changes()` as a plain `Future` over a generation counter and `WAITERS`
  fixed waker slots — Embassy, RTIC and a hand-written poll loop all drive
  it; an evicted waiter is woken, never dropped.
- The `Validate` trait, and `Error`/`ErrorKind` small enough for a status
  register.

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

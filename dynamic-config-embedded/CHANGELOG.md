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

### Changed

- Released in lockstep with `dynamic-config` 0.2.0, where the attribute
  declares and the builder configures. This crate's own surface is
  unchanged; its examples and docs now configure through the builder.

## [0.1.0] — 2026-08-10

### Breaking

- `ConfigCell` gains a const-generic waiter budget:
  `ConfigCell<T, const WAITERS: usize = 4>`. `WAITERS` is renamed
  `DEFAULT_WAITERS`. Un-annotated `let cell = ConfigCell::new()` now needs
  a type annotation; statics are unaffected.

### Fixed

- Trailing bytes after a JSON document are rejected — a reused link buffer
  can no longer smuggle a stale tail into an installed configuration.
- The waiter evicted by a fifth registration really is woken, and the docs
  are honest about steady-state churn beyond the budget.

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

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

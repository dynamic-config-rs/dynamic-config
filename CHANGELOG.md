# Changelog

All notable changes to this project are documented here. The format follows
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

The first release: ten crates, versioned together.

### Added

- **`dynamic-config`** — hot-reloadable configuration behind one attribute:
  layered loading (defaults < files < remote < `.env` < environment <
  bindings < flags < overrides), lock-free reads, file watching with
  debounce, an async surface that names no runtime, remote stores, a
  last-known-good cache, transparent `age` decryption, JSON Schema export,
  provenance (`source_of`), and diagnostics that never contain values.
- **`dynamic-config-macros`** — the `#[dynamic_config]` attribute with twenty
  arguments, every misuse a compile error pointing at the offending token.
- **Seven store crates** — etcd, Consul, NATS, Redis, Vault, S3 and
  Firestore, each a separate crate so one store's dependency tree never
  reaches a build that did not ask for it. All seven watch, all seven are
  tested against real servers in containers.
- **`dynamic-config-embedded`** — the same shape for `no_std` targets: a
  snapshot in a `static`, validation before installation, `changes()` as a
  plain `Future`. No allocator, no runtime, no code shared with the rest —
  deliberately.

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...HEAD
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

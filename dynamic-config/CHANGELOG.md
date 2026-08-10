# Changelog

All notable changes to `dynamic-config` are documented here. The format follows
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

## [0.1.0] — 2026-08-10

### Breaking

- `start_watch()` while already watching → `Err(AlreadyExists)`; watchers
  keyed by `TypeId` (per generic instantiation); `watch::spawn`/`spawn_with`
  take a `TypeId`.
- `changes()` waiters wake before reload hooks run.
- Empty-env rule unified (trim-empty + `allow_empty_env` everywhere,
  bindings included).
- Cache files carry a format marker; 0.0.x files read via a temporary
  fallback.
- A remote fetch overtaken by `set_remote` is discarded.

### Added

- `on_reload_scoped`/`HookGuard`; panic-isolated hooks; `set_defaults`;
  deterministic alias chains + cycle rejection; fsync'd atomic writes;
  honest drift reports (including "could not compare").

### Fixed

- Non-UTF-8 environment no longer panics `load()`; recovery validates,
  seeds the diff baseline and keeps env above `.env`; renamed secrets
  redact correctly; path-shaped profiles refused; debounce bounded and
  pre-filtered; `Fetched` Debug redacted; zeroization covers the encrypted
  *write* path.

## [0.0.1] — 2026-08-10

Initial release.

### Added

- `#[dynamic_config]` — one attribute wiring loading, storage and reload:
  `files`, `name`+`paths` discovery, `key`, `env` (+`nest`,
  `allow_empty_env`), `watch` (+`debounce`, `poll`, `poll_interval`),
  `async`, `profile_env`, `validate`, `diff`, `env_files`, `save`, `schema`,
  `cache` (+`cache_mode`).
- Layered precedence with provenance: `source_of` names the file, variable or
  store a value came from; `check()` reports the whole configuration at once.
- Lock-free reads: `current()` is one atomic load. Reload callbacks,
  `changes()` for async waiters, and a `Group` for types that reload
  together.
- Runtime layers (`set_default` / `set_override`), key aliases, per-field
  environment bindings (`bind_env`), clap integration, `"30s"`/`"64MiB"`
  units.
- Remote stores behind `RemoteSource` / `AsyncRemoteSource`; fetching is
  explicit, so `load()` never touches the network.
- A last-known-good cache with three modes (full / redacted / fingerprint),
  written `0600` and atomically.
- Transparent decryption of `.age` config files, `save_encrypted`, and a
  `Decryptor`/`Encryptor` pair for schemes of your own; decrypted text is
  zeroized on every path.
- JSON Schema export (`schema()`), `.env` files as the environment layer,
  foreign figment providers as sources.
- Diagnostics report paths and types, never values — enforced by its own
  test suite.

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

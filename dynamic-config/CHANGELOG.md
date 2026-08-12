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

## [0.4.0] — 2026-08-12

### Added

- **The figment review's fixes landed.** Top-level tables named `global`
  or `default` are ordinary sections now: sections ride on a *namespaced*
  profile, so figment's reserved-profile inheritance — which silently let
  a `global` table override every section's own values, invisible to
  `check` and `source_of` — has nothing to grab. And environment
  provenance names the exact variable (`APP_DB_POOL__MAX_SIZE`), derived
  from prefix, path and the nesting separator, instead of `APP_DB_*` —
  in errors, `source_of`, snapshot provenance, `check` and `explain`
  alike.
- **`cache_encrypted(path, encryptor)` — the last-known-good cache,
  encrypted at rest.** Full fidelity with nothing readable on disk:
  written through the caller's `Encryptor`, recovered through the
  installed `Decryptor` — the same door `encrypted_file` reads through.
  Behind the `decrypt` feature.
- **`dynamic-config-cli` is on crates.io** (`cargo install
  dynamic-config-cli`), with `completions` and `man` subcommands — and
  `explain` now redacts by default; `--show-values` opts in (the CLI's own
  changelog carries the details).
- **`Dynamic<T>` — the instance engine.** One configuration per *value*
  rather than per type: its own snapshot, hooks, watcher and cache, with
  the same builder carrying the sources. `current()` answers `Option`
  instead of panicking, two instances of one type watch side by side, and
  the type-level surface is untouched. The watcher registry now keys on
  `WatchKey` (`Type` or `Instance`) to make that possible.
- **`Value` and `Snapshot::to_value`.** The resolved tree as owned data —
  seven shapes, no loader types in the signature, built by walking the
  resolved tree rather than a JSON round trip — for boundaries that need
  configuration as values: exporters, language bindings.

### Breaking

- `watch::spawn` / `watch::spawn_with` take a `WatchKey` where they took a
  `TypeId` (`WatchKey::Type(id)` is the old behaviour), and the async and
  grouped-commit builder surfaces (`load_async`, `init_async`, `prepare`)
  require `T: Sync` — the builder can now carry a shared cell, and moving
  it to a worker moves the cell with it.

## [0.3.0] — 2026-08-11

### Breaking

- `apply_remote` → `remote_sink()` + `RemoteSink::apply`: pushes carry the
  generation of the source their loop was wired against, and a replaced
  source's sink refuses. `Remote::install` is no longer public.

### Added

- The concurrency claims are model-checked: under `--cfg loom` the library
  swaps its sync primitives for loom's (`src/sync.rs`), and `just loom`
  runs the remote fence — fetch and push — and the async wake protocol
  through every interleaving, on the real code. The check-register-check
  dance now lives in one place, `Notify::poll_with`, which is what the
  model drives.

## [0.2.0] — 2026-08-11

### Breaking

- **The attribute declares; the builder configures.** `#[dynamic_config]`
  takes no arguments; the `Builder` carries the whole source surface (plus
  `validate`, `watch_with`, `prepare`, `reload`, `schema`, and per-type
  memory of the configuration `init` installed). Generated
  `load`/`init`/`start_watch`/`save*`/`schema` methods are gone; the
  attribute error for any argument is the migration map.
- `watch::spawn` / `spawn_with` take an owned `watch::Watched` instead of a
  `LoadSpec<'static>`.
- The cache moved to `Builder::cache(path, mode)`; the mode is always
  spelled out, and redaction-dependent modes are refused without the
  generated builder's secret knowledge.

### Added

- `explain(path)` / `Explanation` / `Contribution`: per-layer provenance
  tables, secrets pre-redacted in the generated method.
- `Snapshot` carries provenance; `Snapshot::source_of(path)` answers for the
  snapshot in hand.
- `strict_env` (`.strict_env()` on the builder, `with_strict_env` on
  `LoadSpec`): ambiguous environment spellings are refused with the
  variable named.
- `Builder<T>` and the generated `builder()`: runtime-chosen sources that
  load — or install — with the attribute's exact semantics; now with
  `discover`, `cache` (+ recovery), `watch` and async `load`/`init`.
- `changed_paths(old, new)` (audit, paths only); watcher reloads are
  `config_reload` tracing spans with outcome and duration.

### Changed

- `changes()` before `init()` is contract: the initial install is the
  handle's first change.

### Fixed

- `Snapshot::fmt` (and `Recovery` through it) printed resolved values,
  secrets included; both now show keys and shape only.
- The `strict_env` refusal does not echo the offending value.

## [0.1.0] — 2026-08-10

### Breaking

- `Recovery::Drift` carries `Option<Vec<String>>`: `Some` is the fingerprint
  (key paths, or one explanatory sentence when only values moved), `None`
  means the comparison itself was impossible.
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

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

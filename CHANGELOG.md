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

## [0.1.0] — 2026-08-10

A full-workspace hardening release, driven by a three-way external review.
Breaking changes are deliberate and none of them should survive silent
misbehaviour: every one replaces a quiet wrong with a loud right.

### Breaking

- `Recovery::Drift` carries `Option<Vec<String>>`: `Some` is the fingerprint
  (key paths, or one explanatory sentence when only values moved), `None`
  means the comparison itself was impossible.
- `start_watch()` on a type that is already being watched returns
  `Err(AlreadyExists)` instead of a success handle that owned nothing.
- Watchers are keyed by `TypeId`: generic configurations get one watcher per
  instantiation — `Db<Postgres>` and `Db<Mysql>` no longer silently share
  (and lose) one. `watch::spawn`/`spawn_with` take the `TypeId`.
- Async waiters (`changes()`) are woken *before* reload hooks run, so a slow
  hook no longer delays every async reader.
- The empty-environment rule is unified: whitespace-only counts as empty,
  and `allow_empty_env` is honoured by env-var bindings too.
- Last-known-good cache files now carry a `__dynamic_config_cache` marker;
  files written by 0.0.x are still read via a fallback that will be removed
  in the next minor.
- A remote fetch whose source was replaced mid-flight is discarded instead
  of pairing the new source with the old store's document.
- Embedded: `ConfigCell` gains a const-generic waiter budget
  (`ConfigCell<T, 8>`); `WAITERS` is now `DEFAULT_WAITERS`.
- Store watch callbacks that panic end the watch with an error instead of
  killing the calling thread.

### Added

- `on_reload_scoped` → `HookGuard` (drop to unsubscribe); hooks are now
  panic-isolated.
- `set_defaults(&struct)` seeds the defaults layer from a whole struct.
- Alias chains resolve deterministically; cycles are refused at `add`;
  a runtime default no longer defeats an alias.
- Atomic writes fsync before rename (and the parent directory on Unix).
- mdBook documentation, published to GitHub Pages.

### Fixed

- `{:?}` on Vault/Consul/Firestore sources — and on `Fetched` — no longer
  prints credentials or document contents.
- A non-UTF-8 variable anywhere in the environment no longer panics
  `load()`.
- Recovery honours `validate`, seeds the diff baseline, and keeps the real
  environment above `.env` files — all exactly as a normal load does.
- `#[serde(rename)]`d (and `rename_all`'d) secrets are redacted in the
  redacted cache under the names the files actually use.
- A profile from the environment must be a plain word — `APP_ENV=../x`
  is refused, not interpolated into a file path.
- The file watcher filters events before debouncing and bounds the wait, so
  a chatty neighbour cannot starve reloads.
- Redis: a dead subscription ends the watch with an error instead of a
  silent busy-loop; URL redaction survives passwords containing `@`.
- etcd: an expired auth token mid-stream re-logs-in and re-establishes the
  watch instead of failing terminally.
- Embedded: trailing bytes after a JSON document are rejected; the evicted
  waiter really is woken.

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

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

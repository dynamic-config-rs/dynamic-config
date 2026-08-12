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

## [0.5.0] — 2026-08-12

### Added

- **`dynamic-config-py` — the Python bindings.** A PyO3 extension pairing
  this engine with Pydantic: Rust owns sources, layering, watching,
  recovery and provenance; Pydantic owns the schema; Python reads a
  cached model for the price of an attribute lookup. Validation runs once
  per successful resolve, a reload Pydantic rejects keeps the previous
  model serving, and the secret list is derived from the model's own
  `SecretStr` fields. Ships to PyPI as `dynamic-config-py`, on a version
  of its own — the wheel embeds the engine rather than depending on a
  published version of it, so it does not move every time the crates do
  — and the import is `dynamic_config`. See [its chapter](book/src/python.md). Reading is an
  attribute lookup — 28 ns against a module global's 20 — because the
  model is published into the Python object as it installs;
  `changed_paths` gives the audit half of a reload from Python too.
  Every blocking call has an async twin (`init_async`, `load_async`,
  `reload_async`, `changed_async`) and an executor knob to choose which
  pool pays for it, thirteen runnable examples ship with the package, and the chapter has its own sections for
  [async](book/src/python/async.md),
  [data types](book/src/python/types.md),
  [web frameworks](book/src/python/frameworks.md) and
  [limitations](book/src/python/limitations.md).
- `Builder::validate` accepts closures, not just function pointers: a
  validator that needs *context* — a policy object, a schema, another
  runtime's validator — could not be written as a `fn`, and that is the
  shape a language binding needs. A plain `fn` still coerces, so every
  existing call site is unchanged.
- `touches_secret(path, secrets)`, the one rule every redaction door
  asks: a path that *is*, sits *under*, or *contains* a secret is
  redacted.

### Fixed

- **Nested secrets were redacted nowhere.** A secret named by a dotted
  path — `credentials.password`, which is what a nested model produces —
  was missed by both redaction doors: `explain` matched only the head of
  the path, and the redacted last-known-good cache dropped only
  top-level keys. Both now understand dotted paths, so a secret inside a
  nested table stays out of the cache file and reads `***` in an
  explanation, whether the path asked about is the secret, something
  under it, or the table containing it.

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

- The remote push path is fenced like the fetch path always was:
  `apply_remote(document)` is replaced by `remote_sink()` — taken **once,
  at wiring** — whose `apply` refuses to deliver for a source that has
  since been replaced. A stale watch loop's push now bounces (and, since a
  callback error ends a store watch, the stale loop winds itself down)
  instead of quietly overwriting the store that followed it.

### Added

- The concurrency claims are model-checked: under `--cfg loom` the library
  swaps its sync primitives for loom's (`src/sync.rs`), and `just loom`
  runs the remote fence — fetch and push — and the async wake protocol
  through every interleaving, on the real code. The check-register-check
  dance now lives in one place, `Notify::poll_with`, which is what the
  model drives.

### Security

- The dependency lockfile moved to the patched versions behind every open
  Dependabot alert: `quinn-proto` 0.11.15 (GHSA-4w2j-m93h-cj5j,
  GHSA-6xvm-j4wr-6v98), `actix-http` 3.12.1 (GHSA-xhj4-vrgc-hr34),
  `serde_with` 3.21.0 (GHSA-7gcf-g7xr-8hxj) and `aws-sdk-s3` 1.112.0
  (GHSA-g59m-gf8j-gjf5). Library consumers resolve their own trees and
  were never pinned to the vulnerable versions by these crates; the
  lockfile governs this repository's CI and any `--locked` install. The
  standing triage rule is now written into `SECURITY.md`.

## [0.2.0] — 2026-08-11

### Breaking

- **The attribute declares; the builder configures.** `#[dynamic_config]`
  takes no arguments any more: every source argument moved to the `Builder`
  — `files` → `.file(..)`, `name`+`paths` → `.discover(..)`, `key` →
  `builder("key")`, `env`/`nest`/`allow_empty_env`/`strict_env` →
  `.env(..)`/`.nest(..)`/`.allow_empty_env()`/`.strict_env()`, `env_files`
  → `.env_file(..)`, `profile_env` → `.profile_env(..)`,
  `cache`/`cache_mode` → `.cache(path, mode)`, `validate` →
  `.validate(f)`, `watch`/`debounce`/`poll` → `.watch(debounce)` /
  `.watch_with(debounce, mode)` on the builder `init()` was called on.
  Generated `load`/`init`/`start_watch`/`save*`/`schema` methods are gone:
  loading goes through the builder, `save`/`save_new`/`save_encrypted` are
  the free functions they always also were, and `schema` is
  `builder.schema()`. A successful `init` remembers its builder, which is
  how `source_of`, `check`, `explain`, `prepare`, `apply_remote` and the
  async loaders on the type keep answering. The attribute error for any
  argument is the migration map. The `diff` argument is gone —
  `changed_paths` in an `on_reload` hook is its replacement.
- `watch::spawn` / `spawn_with` take an owned `watch::Watched` (built with
  `Watched::from_spec`) instead of a `LoadSpec<'static>` — the watch no
  longer requires statics only the attribute can produce, which is what
  frees the builder (or anything else) to start one from runtime data.
- The cache is configured on the builder, and the mode is always spelled
  out: `.cache(path, CacheMode::Redacted)`. Secrets on disk are a decision,
  not a side effect — `Redacted` recovers completely when the secrets
  arrive from somewhere live, and the redaction-dependent modes are refused
  on a bare `Builder::new`, which cannot know which fields are secret.

### Added

- `explain(path)` — every configured layer's answer for one path, not just
  the winner's, rendered as a table; the one diagnostic that shows values —
  through `Display`, deliberately: its `Debug` is value-free — and
  `#[config(secret)]` fields stay `***`. Generated on every config type
  and available as `dynamic_config::explain` without the macro.
- `Snapshot::source_of(path)` — snapshots now carry the provenance of their
  own leaves, captured at resolution time; the free `source_of()` keeps its
  next-load meaning, now documented as such.
- `dynamic-config-cli` (Experimental, in-repo, not yet published): `explain`
  and `diff` from a shell, the load restated as flags.
- `strict_env` (`.strict_env()` on the builder, `with_strict_env` on
  `LoadSpec`): the yes/no/on/off family in an environment value (or a
  `.env` file) becomes an error naming the variable, instead of arriving as
  a string where a boolean was meant — and the refusal holds on the cache
  recovery path too. Loose parsing stays the default.
- `Builder<T>`: runtime-chosen sources with the attribute's semantics —
  `Builder::new("db").file(path).env("APP_").load()`, no macro required. On
  a `#[dynamic_config]` type the generated `builder()` adds `init()`, which
  installs into the same snapshot `current()` reads. The builder reaches
  source-side parity with the attribute: `discover(name, paths)`,
  `cache(path, mode)` with last-known-good recovery (redaction-dependent
  modes are refused unless the generated `builder()` supplies the secret
  fields — and `Fingerprint` never recovers, even from a value-bearing
  file an earlier deployment left at the same path), `watch(debounce)`
  through the same one-watcher-per-type registry, and
  `load_async`/`init_async` under the `async` feature.
- Reload observability: under `tracing`, every watcher reload is a
  `config_reload` span with outcome and duration; the stderr lines carry
  the duration without it. `changed_paths(old, new)` names what moved
  between two configuration values — paths only, never values — for audit
  logging inside `on_reload`.
- The book gains [The Reload Lifecycle](book/src/reload-lifecycle.md):
  where the crate's half of a reload ends, and the surface for yours — and
  [The Builder, Feature by Feature](book/src/builder-tour.md): every
  capability with a minimal example, files to callbacks to hot reload.

### Changed

- `changes()` before `init()` is now contract: the handle has seen nothing,
  so the initial install is its first change — "wake me when configuration
  exists". The behaviour is unchanged; it is now documented and tested.

### Fixed

- `Snapshot`'s `Debug` printed the resolved values — secrets included — and
  so did `Recovery`'s through it. Both now describe keys and shape only,
  with a test that plants a secret and greps the output.
- The `strict_env` refusal does not echo the offending value; it names the
  variable and the ambiguous family. No diagnostic prints a value, without
  exception.

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

[Unreleased]: https://github.com/ctolon/dynamic-config/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

<div align="center">

# dynamic-config

**Hot-reloadable, layered configuration for Rust — one attribute, lock-free reads.**

[![CI](https://github.com/dynamic-config-rs/dynamic-config/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/dynamic-config-rs/dynamic-config/actions/workflows/ci.yml)
[![Security](https://github.com/dynamic-config-rs/dynamic-config/actions/workflows/security.yml/badge.svg?branch=main)](https://github.com/dynamic-config-rs/dynamic-config/actions/workflows/security.yml)
[![crates.io](https://img.shields.io/crates/v/dynamic-config.svg)](https://crates.io/crates/dynamic-config)
[![docs.rs](https://img.shields.io/docsrs/dynamic-config)](https://docs.rs/dynamic-config)
[![MSRV](https://img.shields.io/badge/MSRV-1.71-blue)](https://dynamic-config-rs.github.io/msrv-features.html)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/dynamic-config-rs/dynamic-config/badge)](https://scorecard.dev/viewer/?uri=github.com/dynamic-config-rs/dynamic-config)

[**The Book**](https://dynamic-config-rs.github.io/) · [API docs](https://docs.rs/dynamic-config) · [Examples](https://github.com/dynamic-config-rs/dynamic-config/tree/main/dynamic-config/examples) · [Changelog](CHANGELOG.md)

</div>

---

Configuration that stays live after startup: files, environment, remote
stores and command-line flags merged into one typed struct, re-read when
they change, served to every thread as one atomic load.

```toml
[dependencies]
dynamic-config = { version = "0.7.0", features = ["toml", "watch"] }
```

```rust
use dynamic_config::dynamic_config;
use serde::Deserialize;
use std::time::Duration;

#[dynamic_config]
#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let builder = DatabaseConfig::builder("db")
        .file("config.toml")
        .file("secrets.json")
        .env("APP_");

    builder.init()?;                                     // load once, fail fast on a bad config
    builder.watch(Duration::from_millis(250))?.detach(); // reload in the background from now on

    let config = DatabaseConfig::current();              // one atomic load, on any thread
    println!("{}:{}", config.host, config.port);

    Ok(())
}
```

The attribute declares — *this type is a configuration* — and generates its
storage and accessors. The builder configures: where the sources are is
runtime data, and it lives in runtime code.

## Why this one

- **Reads are lock-free and allocation-free.** `current()` acquires an
  `arc-swap` guard — **85 instructions**, ~20 ns, and zero allocations per
  100 000 reads, all three measured rather than asserted — so configuration
  can be read per request without a second thought.
- **A bad edit cannot take the process down.** A file that no longer parses
  or validates degrades to "no change"; the previous snapshot keeps serving,
  and the error is reported.
- **Layers with provenance.** `defaults < discovered < files < remote <
  secrets_dir < .env < environment < bindings < flags < overrides` — and
  `source_of("key")` names the file, variable or store a value actually
  came from.
- **Secrets stay out of diagnostics.** Errors, diffs, reports and `{:?}`
  print paths and types, never values — enforced by its own test suite.
- **Any runtime, or none.** The async surface is a `Future` and a thread;
  tokio, smol and Embassy all drive it. Blocking work never lands on your
  executor.
- **Remote stores are explicit.** `refresh_remote()` does the network round
  trip; `load()` never does. Eight store crates ship from
  [dynamic-config-remote](https://github.com/dynamic-config-rs/dynamic-config-remote),
  each watching the way its protocol allows — seven over a network, and git.

The full story — precedence, profiles, discovery, hot reload, encryption,
schema export, units, the last-known-good cache, testing patterns — lives in
[**the book**](https://dynamic-config-rs.github.io/).

## This repository, and the family

| Crate | What | Stability |
|---|---|---|
| [`dynamic-config`](https://crates.io/crates/dynamic-config) | the engine: loading, layers, storage, watching | **Beta** |
| [`dynamic-config-macros`](https://crates.io/crates/dynamic-config-macros) | `#[dynamic_config]` | **Beta** |
| [`dynamic-config-embedded`](https://crates.io/crates/dynamic-config-embedded) | the same shape for `no_std` targets | **Beta** |
| [`dynamic-config-cli`](https://crates.io/crates/dynamic-config-cli) | `explain` and `diff` on the command line — `cargo install dynamic-config-cli` | **Beta** |

The rest of the family is released from its own repository, each naming
this engine with a caret so a patch here reaches it without a release
there:

| Repository | What it ships |
|---|---|
| [dynamic-config-remote](https://github.com/dynamic-config-rs/dynamic-config-remote) | eight store crates — etcd, Consul, NATS, Redis, Vault, S3, Firestore, git — and `dynamic-config-server` |
| [dynamic-config-python](https://github.com/dynamic-config-rs/dynamic-config-python) | `pip install dynamic-config-py`; a dataclass, Pydantic or msgspec validates |
| [dynamic-config-node](https://github.com/dynamic-config-rs/dynamic-config-node) | `npm install dynamic-config-node`; Zod, Ajv or a function of your own validates |

**Every crate is Beta**: breaking changes bump the minor pre-1.0 and are
announced in the changelog; a patch never breaks.

**Between here and 1.0, only security fixes and hotfixes land.** The
surface is what it is going to be for 0.x: no new sources, no new stores,
no new methods on the settled types. Pin the minor version and take
patches automatically. Details in
[Stability Tiers](https://dynamic-config-rs.github.io/stability-tiers.html).

## MSRV

| | floor |
|---|---|
| `dynamic-config` core | **1.71** |
| `schema` feature | 1.74 (schemars) |
| `watch` / `age` / `full` features | 1.85 (measured, not declared) |
| `dynamic-config-cli` | 1.85 |
| `dynamic-config-embedded` | 1.83 |

The store crates, the server and the bindings declare their own floors, in
their own repositories — a companion pays for what it pulls in.

MSRV changes are breaking. Every floor has a CI row against a real
toolchain; the full table with reasons is in
[MSRV & Features](https://dynamic-config-rs.github.io/msrv-features.html).

## Contributing

[CONTRIBUTING.md](https://github.com/dynamic-config-rs/dynamic-config/blob/main/CONTRIBUTING.md) is the short version;
[the onboarding tour](https://github.com/dynamic-config-rs/dynamic-config/blob/main/docs/CONTRIBUTOR-ONBOARDING.md) walks every module.
What will *not* be built, and why, is in
[Limitations & Not Planned](https://dynamic-config-rs.github.io/limitations.html);
what might be is in [ROADMAP.md](https://github.com/dynamic-config-rs/dynamic-config/blob/main/ROADMAP.md).

## Credits

What this engine is built on and whose ideas it took —
[CREDITS.md](https://github.com/dynamic-config-rs/dynamic-config/blob/main/CREDITS.md).

## License

[MIT](LICENSE).

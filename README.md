<div align="center">

# dynamic-config

**Hot-reloadable, layered configuration for Rust — one attribute, lock-free reads.**

[![CI](https://github.com/ctolon/dynamic-config/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ctolon/dynamic-config/actions/workflows/ci.yml)
[![Security](https://github.com/ctolon/dynamic-config/actions/workflows/security.yml/badge.svg?branch=main)](https://github.com/ctolon/dynamic-config/actions/workflows/security.yml)
[![crates.io](https://img.shields.io/crates/v/dynamic-config.svg)](https://crates.io/crates/dynamic-config)
[![docs.rs](https://img.shields.io/docsrs/dynamic-config)](https://docs.rs/dynamic-config)
[![MSRV](https://img.shields.io/badge/MSRV-1.71-blue)](https://ctolon.github.io/dynamic-config/msrv-features.html)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/ctolon/dynamic-config/badge)](https://scorecard.dev/viewer/?uri=github.com/ctolon/dynamic-config)

[**The Book**](https://ctolon.github.io/dynamic-config/) · [API docs](https://docs.rs/dynamic-config) · [Examples](https://github.com/ctolon/dynamic-config/tree/main/dynamic-config/examples) · [Changelog](CHANGELOG.md)

</div>

---

Configuration that stays live after startup: files, environment, remote
stores and command-line flags merged into one typed struct, re-read when
they change, served to every thread as one atomic load.

```toml
[dependencies]
dynamic-config = { version = "0.3.0", features = ["toml", "watch"] }
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

- **Reads are lock-free.** `current()` is an atomic pointer load — ~17 ns —
  so configuration can be read per request without a second thought.
- **A bad edit cannot take the process down.** A file that no longer parses
  or validates degrades to "no change"; the previous snapshot keeps serving,
  and the error is reported.
- **Layers with provenance.** `defaults < discovered < files < remote <
  .env < environment < bindings < flags < overrides` — and `source_of("key")`
  names the file, variable or store a value actually came from.
- **Secrets stay out of diagnostics.** Errors, diffs, reports and `{:?}`
  print paths and types, never values — enforced by its own test suite.
- **Any runtime, or none.** The async surface is a `Future` and a thread;
  tokio, smol and Embassy all drive it. Blocking work never lands on your
  executor.
- **Remote stores are explicit.** `refresh_remote()` does the network round
  trip; `load()` never does. Seven store crates ship, each watching the way
  its protocol allows.

The full story — precedence, profiles, discovery, hot reload, encryption,
schema export, units, the last-known-good cache, testing patterns — lives in
[**the book**](https://ctolon.github.io/dynamic-config/).

## The workspace

| Crate | What | Stability |
|---|---|---|
| [`dynamic-config`](https://crates.io/crates/dynamic-config) | the engine: loading, layers, storage, watching | **Beta** |
| [`dynamic-config-macros`](https://crates.io/crates/dynamic-config-macros) | `#[dynamic_config]` | **Beta** |
| [`dynamic-config-etcd`](dynamic-config-etcd) | etcd, push watch over gRPC | Experimental |
| [`dynamic-config-consul`](dynamic-config-consul) | Consul KV, blocking queries | Experimental |
| [`dynamic-config-nats`](dynamic-config-nats) | NATS JetStream KV, push watch | Experimental |
| [`dynamic-config-redis`](dynamic-config-redis) | Redis, keyspace notifications | Experimental |
| [`dynamic-config-vault`](dynamic-config-vault) | Vault KV v2, version polling | Experimental |
| [`dynamic-config-s3`](dynamic-config-s3) | S3 & compatibles, ETag polling — needs tokio | Experimental |
| [`dynamic-config-firestore`](dynamic-config-firestore) | Firestore REST, `updateTime` polling | Experimental |
| [`dynamic-config-embedded`](dynamic-config-embedded) | the same shape for `no_std` targets | Experimental |
| [`dynamic-config-cli`](dynamic-config-cli) | `explain` and `diff` on the command line — in-repo, not yet published | Experimental |

**Beta**: breaking changes bump the minor pre-1.0 and are announced in the
changelog. **Experimental**: may change shape without ceremony — pin an
exact version. Details in
[Stability Tiers](https://ctolon.github.io/dynamic-config/stability-tiers.html).

Every store follows the same contract — the current value is not announced
at startup, a deleted key is not a change, transport failures retry, a
panicking callback ends the watch with an error — and each documents its
stop latency and change-detection rule side by side in
[Store Crates at a Glance](https://ctolon.github.io/dynamic-config/remote-stores/store-crates.html).

## MSRV

| | floor |
|---|---|
| `dynamic-config` core | **1.71** |
| `schema` feature | 1.74 (schemars) |
| `watch` / `age` / `full` features | 1.85 (measured, not declared) |
| store crates | 1.85 — nats/redis/s3: 1.88 (their clients) |
| `dynamic-config-cli` | 1.85 |
| `dynamic-config-embedded` | 1.83 |

MSRV changes are breaking. Every floor has a CI row against a real
toolchain; the full table with reasons is in
[MSRV & Features](https://ctolon.github.io/dynamic-config/msrv-features.html).

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) is the short version;
[the onboarding tour](docs/CONTRIBUTOR-ONBOARDING.md) walks every module.
What will *not* be built, and why, is in
[Limitations & Not Planned](https://ctolon.github.io/dynamic-config/limitations.html);
what might be is in [ROADMAP.md](ROADMAP.md).

## License

[MIT](LICENSE).

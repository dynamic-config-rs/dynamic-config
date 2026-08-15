# Quick Start

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

    builder.init()?;                                  // load once, fail fast on a bad config
    builder.watch(Duration::from_millis(250))?.detach(); // reload in the background from now on

    let config = DatabaseConfig::current();
    println!("{}:{}", config.host, config.port);

    Ok(())
}
```

```toml
[dependencies]
dynamic-config = { version = "<version>", features = ["toml", "watch"] }
```

`<version>` stands for the latest release —
[crates.io](https://crates.io/crates/dynamic-config) names it, and every
snippet in this book uses the same placeholder so the book cannot go stale
one number at a time. Which features to turn on is the
[Cargo Features](features.md) chapter's whole subject.

The attribute declares — *this type is a configuration* — and generates its
storage and accessors. The builder configures: which files, which prefix,
whether to watch. That split is the crate's shape, and the
[Attribute Reference](attribute-reference.md) is the map of both halves.

## Features

Every one of these is described in full in the chapters that follow; this is
the map.

### Loading

| | |
|---|---|
| **Formats** | JSON, TOML, YAML — each behind its own feature, and a file whose feature is off is a load-time error naming the feature to add |
| **Several files, merged** | `.file("config.toml").file("secrets.json")`, in call order; a file that is not there is skipped, which is what makes an optional `secrets.json` work |
| **Discovery** | `.discover("config", ["/etc/myapp", "~/.config/myapp", "."])`; `~` expands, and resolution happens per load so a file that appears later is picked up |
| **Profiles** | `.profile_env("APP_ENV")` layers `config.production.toml` over `config.toml`, for discovered and listed files alike |
| **Encrypted files** | `secrets.json.age` decrypts at load time; the suffix marks it, the extension under it names the format |
| **`.env` files** | `.env_file(".env")`, read as the environment layer rather than as documents — and without touching the process environment |
| **Any figment provider** | `Source::provider(..)` behind the `figment` feature, for `Serialized::defaults(T)`, a custom `Env`, or one you wrote |
| **No files at all** | a builder with no `.file(..)` calls, for a container fed by a store and the environment |

### Layers

```text
defaults < files < remote < secrets_dir < .env < APP_DB_* < bind_env < flags < overrides
```

| | |
|---|---|
| **Environment** | `.env("APP_")` with configurable nesting (`APP_DB_POOL__MAX_SIZE`), and `FOO=` treated as unset unless you say otherwise |
| **Named variables** | `bind_env("port", "PORT")` for the ones you do not get to name — `PORT`, `DATABASE_URL`, `REDIS_URL` |
| **Command line** | `set_flag`, `set_assignments(["k=v"])`, and `bind_clap` behind a feature that takes only arguments that really came from the command line |
| **Runtime** | `set_default` below everything, `set_override` above it |
| **Key aliases** | `alias("pool.size", "pool.max_size")` keeps files written before a rename working, filling a gap rather than overriding |
| **Tables merge, arrays replace** | a three-line `secrets.json` overrides two fields of a large `config.toml`; a list is never silently concatenated |

### Reading

| | |
|---|---|
| **Lock-free** | `current()` is an atomic load — no mutex, no contention, callable per request |
| **Snapshots** | a reader holding an `Arc` keeps its own generation; a reload never mutates underneath it |
| **Generic config types** | `Db<Postgres>` and `Db<Mysql>` get separate snapshots, keyed by `TypeId`; non-generic types keep their `static` and pay nothing |
| **Schema-less access** | `snapshot()` plus `get`, `contains` and `sub`, for the keys a struct does not name |

### Reloading

| | |
|---|---|
| **File watching** | directory-level, so editor and `mv`-based atomic saves survive; Kubernetes ConfigMap updates are recognised |
| **Poll fallback** | `watch_with(.., WatchMode::Poll { .. })` for NFS and overlay filesystems, where inotify registers and then silently delivers nothing |
| **Debounce** | one editor save is several filesystem events; `watch(debounce)` collapses them into one reload |
| **Remote stores** | etcd, Consul, NATS, Redis, Vault, S3 and Firestore — each watching the way its protocol allows |
| **Hooks** | `on_reload(previous, current)`, and `changes()` for a task that would rather await |
| **Any runtime, or none** | `changes()` is a `Future` over a generation counter and a list of wakers; [tokio](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/tokio_runtime.rs), [smol](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/smol_runtime.rs) and [Embassy](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/embassy_runtime.rs) all drive it |
| **All-or-nothing** | `ReloadGroup` prepares every member before any of them commits |
| **Key-level diffs** | `changed_paths(old, new)` reports which keys moved — paths only, never values |

### Safety

| | |
|---|---|
| **Validation** | `.validate(f)` runs your own check on every load; a reload that fails it keeps the previous snapshot |
| **A bad reload cannot take the process down** | the running snapshot stays until a new one is complete and valid |
| **Secret redaction** | `#[config(secret)]` prints `***`, and `#[derive(Debug)]` alongside it is a compile error rather than a race between two impls |
| **Nothing leaks a value** | diffs, `check()` reports, unknown-key suggestions and error messages all report paths and types, never values |
| **Files written are private** | `save` and the cache *create* their file `0600` and refuse to follow a symlink planted at the temporary path |
| **Writing without replacing** | `save_new` refuses if the file exists, for a setup wizard that must not overwrite what somebody wrote |
| **Writing encrypted** | `save_encrypted` to a recipient list, the counterpart to reading a `secrets.json.age` |
| **Last known good** | `.cache(path, mode)` starts from yesterday's configuration when today's is broken, in three modes so what lands on disk is a choice |

### Diagnostics

| | |
|---|---|
| **Provenance in every error** | `pool.max_size: invalid type: found a string, expected u16 (from APP_DB_)` |
| **`source_of` / `is_set`** | which layer supplies a key, and whether anything does |
| **`check()`** | what the configuration resolves to, without loading it — works when the load fails, which is when it is worth running |
| **Unknown keys** | with suggestions from a transposition-aware edit distance, so `prot` finds `port` |
| **A JSON Schema** | `builder.schema()` describes the *file*, marks secrets `writeOnly`, and drops `required` because a file is one layer of six |

Every capability, one by one with a minimal example each, is
[The Builder, Feature by Feature](builder-tour.md).

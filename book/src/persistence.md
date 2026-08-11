# Persistence & Writing

## `save`

```rust
use dynamic_config::{save, Format};

save(&config, "config.toml", Format::Toml, "db")?;
```

`save` is a free function taking any `Serialize` value: writing a file is an
operation on a value, not a property of a type, so nothing needs to be
declared to use it. The output is nested under the given key, so what comes
out can be read straight back in by a builder with the same key. Written
through a temporary file and renamed, because the watcher is very likely
watching that directory and a partial file would look like a broken edit.

**Secrets are written in the clear** — `#[config(secret)]` keeps a value out
of logs, not out of a file the program was asked to write. On Unix the file is
created `0600`.

## `save_new` and `save_encrypted`

Two companions to `save`, with the same atomic write:

- `save_new(&value, path, format, key)` — the same, refusing if the file
  exists, for a setup wizard that must not overwrite what somebody wrote.
- `save_encrypted(&value, path, format, key, &encryptor)` — the same,
  encrypted to a recipient list, the counterpart to reading a
  `secrets.json.age`. Requires the `decrypt` feature; see
  [Encryption](encryption.md).

`save` and the cache *create* their file `0600` and refuse to follow a symlink
planted at the temporary path.

## `cache`

```rust
use dynamic_config::CacheMode;

DbConfig::builder("db")
    .file("config.toml")
    .cache("/var/lib/app/last.json", CacheMode::Redacted)
    .init()?;
```

Writes the resolved configuration to that path after every clean `init()` and
every clean reload, and reads it back if a *cold start* fails. A failed **reload** never touches it — a
running process already has something better to fall back on, the snapshot it is
currently serving.

Recovery is loud: it logs a warning naming what failed, because a service quietly
running on yesterday's configuration is its own kind of outage. See
[Last known good](#last-known-good) for what ends up on disk.

## Cache modes

The second argument to `.cache(path, mode)` is a `CacheMode`:
`CacheMode::Redacted`, `CacheMode::Full` or `CacheMode::Fingerprint`. An enum
rather than a string, so a typo is a compile error rather than a refused load.
`Redacted` is the choice to reach for first: the cache is written on every
clean load, and secrets landing on disk should be a decision, not a side
effect — writing them is what `Full` spells out. See
[Last known good](#last-known-good).

`Redacted` and `Fingerprint` need to know which fields are secret, which only
the generated `builder()` on a `#[dynamic_config]` type carries — on a bare
`Builder::new`, those modes are refused at `init` rather than silently caching
everything. `CacheMode::Full`, spelled out, still works there.

## Last known good

A process that cannot read its configuration should normally refuse to start —
that is the point of failing loudly. There is one case where refusing is worse:
a machine reboots, something on disk is half-written or a mount has not appeared
yet, and a service that would otherwise have come up sits dead until a person
notices.

```rust
DbConfig::builder("db")
    .file("/etc/app/config.toml")
    .cache("/var/lib/app/last.json", CacheMode::Redacted)
    .init()?;
```

Every clean `init()` and reload writes the resolved configuration there. A cold start that
fails reads it back, logs a warning naming the failure, and runs. It is opt-in,
and deliberately loud.

Recovery lives in `init()`, not `load()`. `load()` is the pure one — it reads the
sources and hands back a value — and a function that quietly returned yesterday's
answer instead would be a poor thing to build anything else on.

### What ends up on disk

A resolved configuration holds every value, including the ones
`#[config(secret)]` exists to keep out of logs. There is no way to make that not
a trade-off, so it is a choice with three answers rather than a default nobody
was told about.

| `CacheMode` | On disk | Recovers | For |
|---|---|---|---|
| `Full` | everything, secrets included | completely | a host you already trust with the secrets — they were in memory anyway |
| `Redacted` | everything except `#[config(secret)]` fields | only if the secrets arrive from somewhere live | secrets injected through the environment, which is the shape most deployments already have |
| `Fingerprint` | a hash and the key names — no value anywhere | never | somewhere no value may be written, when the diagnosis is still worth having |

On Unix the file is written `0600`. That is the most that can be done without
refusing the request; the rest is documented rather than solved.

`Fingerprint` does not pretend it can recover — a failed start still fails.
What it buys is *which keys have moved since the last time this worked*, which is
usually the first thing anyone wants:

```text
[dynamic-config] DbConfig: cannot start: no such file (/etc/app/config.toml).
Since the last good configuration: pool.max_size is gone, pool.max is new
```

The drift goes to the log rather than into the returned error, because the error
belongs to the caller's own handling and this is a note for whoever reads the
logs afterwards.

### Recovery reads no files

The files are what broke. Recovery loads from the cache plus the environment,
flags and runtime overrides — never from the sources whose failure caused it,
because a malformed file fails to parse whatever sits underneath it. The
environment still wins over the cache, so a `Redacted` cache and
`APP_DB_PASSWORD` recover between them. A configured `.validate(f)` still runs
on the recovered value — yesterday's configuration has to meet today's rules.

### Only a cold start

A failed **reload** never consults the cache. A running process already has
something better than yesterday's configuration to fall back on: the snapshot it
is currently serving.

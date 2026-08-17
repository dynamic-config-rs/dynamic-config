# The Last-Known-Good Cache


A process that cannot read its configuration should normally refuse to start —
which is what failing loudly is for. There is one case where refusing is worse:
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
and loud.

Recovery lives in `init()`, not `load()`. `load()` is the pure one — it reads the
sources and hands back a value — and a function that quietly returned yesterday's
answer instead would be a poor thing to build anything else on.

## What ends up on disk

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

## Recovery reads no files

The files are what broke. Recovery loads from the cache plus the environment,
flags and runtime overrides — never from the sources whose failure caused it,
because a malformed file fails to parse whatever sits underneath it. The
environment still wins over the cache, so a `Redacted` cache and
`APP_DB_PASSWORD` recover between them. A configured `.validate(f)` still runs
on the recovered value — yesterday's configuration has to meet today's rules.

## Only a cold start

A failed **reload** never consults the cache. A running process already has
something better than yesterday's configuration to fall back on: the snapshot it
is currently serving.

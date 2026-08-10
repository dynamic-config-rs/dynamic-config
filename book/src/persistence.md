# Persistence & Writing

## `save`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", save)]
#[derive(Deserialize, Serialize)]
```

Generates `save(&self, path)`. The format comes from the extension and the output
is nested under `key`, so what comes out can be read straight back in. Written
through a temporary file and renamed, because the watcher is very likely
watching that directory and a partial file would look like a broken edit.

Requires `Self: Serialize`. **Secrets are written in the clear** —
`#[config(secret)]` keeps a value out of logs, not out of a file the program was
asked to write. On Unix the file is created `0600`.

## `save_new` and `save_encrypted`

Two companions to `save`, with the same atomic write:

- `save_new(&self, path)` — the same, refusing if the file exists, for a setup
  wizard that must not overwrite what somebody wrote.
- `save_encrypted(&self, path, &encryptor)` — the same, encrypted to a recipient
  list, the counterpart to reading a `secrets.json.age`. Requires the `save`
  argument and the `decrypt` feature; see [Encryption](encryption.md).

`save` and the cache *create* their file `0600` and refuse to follow a symlink
planted at the temporary path.

## `cache`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", cache = "/var/lib/app/last.json")]
```

Writes the resolved configuration to that path after every successful load, and
reads it back if a *cold start* fails. A failed **reload** never touches it — a
running process already has something better to fall back on, the snapshot it is
currently serving.

Recovery is loud: it logs a warning naming what failed, because a service quietly
running on yesterday's configuration is its own kind of outage. See
[Last known good](#last-known-good) for what ends up on disk.

## `cache_mode`

```rust
#[dynamic_config(
    files = ["config.toml"],
    key = "db",
    cache = "/var/lib/app/last.json",
    cache_mode = "redacted",
)]
```

`"full"` (the default), `"redacted"` or `"fingerprint"`. Anything else is a
compile error listing the three. See [Last known good](#last-known-good).

## Last known good

A process that cannot read its configuration should normally refuse to start —
that is the point of failing loudly. There is one case where refusing is worse:
a machine reboots, something on disk is half-written or a mount has not appeared
yet, and a service that would otherwise have come up sits dead until a person
notices.

```rust
#[dynamic_config(files = ["/etc/app/config.toml"], key = "db", cache = "/var/lib/app/last.json")]
```

Every successful load writes the resolved configuration there. A cold start that
fails reads it back, logs a warning naming the failure, and runs. It is opt-in,
and deliberately loud.

Recovery lives in `init()`, not `load()`. `load()` is the pure one — it reads the
sources and hands back a value — and a function that quietly returned yesterday's
answer instead would be a poor thing to build anything else on.

### What ends up on disk

A resolved configuration holds every value, including the ones
`#[config(secret)]` exists to keep out of logs. There is no way to make that not
a trade-off, so it is a choice with three answers rather than a default nobody
was told about — and the default is *write it anyway*, because a cache that
cannot recover is a cache that will disappoint somebody at three in the morning.

| `cache_mode` | On disk | Recovers | For |
|---|---|---|---|
| `"full"` *(default)* | everything, secrets included | completely | a host you already trust with the secrets — they were in memory anyway |
| `"redacted"` | everything except `#[config(secret)]` fields | only if the secrets arrive from somewhere live | secrets injected through the environment, which is the shape most deployments already have |
| `"fingerprint"` | a hash and the key names — no value anywhere | never | somewhere no value may be written, when the diagnosis is still worth having |

On Unix the file is written `0600`. That is the most that can be done without
refusing the request; the rest is documented rather than solved.

`"fingerprint"` does not pretend it can recover — a failed start still fails.
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
environment still wins over the cache, so a `"redacted"` cache and
`APP_DB_PASSWORD` recover between them.

### Only a cold start

A failed **reload** never consults the cache. A running process already has
something better than yesterday's configuration to fall back on: the snapshot it
is currently serving.

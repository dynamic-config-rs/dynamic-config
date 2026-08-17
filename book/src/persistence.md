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
running on yesterday's configuration is its own kind of outage. See [The Last-Known-Good Cache](last-known-good.md) for what ends up on
disk.

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

### Encrypted, the fourth answer

With the `decrypt` feature, `cache_encrypted(path, encryptor)` collapses
the trade-off the three modes exist to navigate: full fidelity — recovery
needs nothing from the live environment — with nothing readable on disk.

```rust
AppConfig::builder("app")
    .file("config.toml")
    .cache_encrypted("/var/lib/app/last.json.age", encryptor)
    .init()?;
```

The cache is written through the `Encryptor` the caller constructs — the
recipients live there, at the call site that owns them — and recovered
through the installed [`Decryptor`](encryption.md), the same door
`encrypted_file(..)` reads through, so one `set_decryptor` covers both.
The path carries the format under the encryption suffix, exactly like an
encrypted source file: `last.json.age` is JSON.

## Last known good

The cache's other half is recovery: what `.cache(path, mode)` writes is what
a *cold start* falls back to when the sources cannot be read at all.

That is its own chapter — [The Last-Known-Good
Cache](last-known-good.md) — covering what ends up on disk in each mode,
when recovery runs and when it does not, and why a failed reload never
touches the file.

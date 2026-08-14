# The Builder, Feature by Feature

One declared type, every capability in turn — a minimal example and the
reason it exists, from the first file to the last callback. Deep dives live
in each feature's own chapter; this page is the map with all the roads on
it.

Everything below starts from the same declaration:

```rust
use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct AppConfig {
    host: String,
    port: u16,
}
```

The attribute takes no arguments: it declares that the type *is* a
configuration and generates its storage. Where the configuration comes from
is the builder's business, at runtime, where sources actually vary.

## Files

```rust
AppConfig::builder("app")
    .file("config.toml")
    .file("secrets.json")
    .init()?;
```

`"app"` is the section key — the top-level table this struct maps onto.
Files merge in call order, later wins, tables merge key by key: a
three-line `secrets.json` overrides two fields of a large `config.toml`
without restating the rest. A missing file is skipped, which is what makes
an optional secrets file work. The format comes from the extension at load
time.

## A document with no section header

```rust
AppConfig::builder("app")
    .whole_document()
    .file("app.json")          // {"host": "0.0.0.0", "port": 8000}
    .init()?;
```

The default reading is one file, several sections: every top-level key
names one, which is what lets a `config.toml` hold `[db]` and `[server]`
for two types that know nothing about each other. A file that is *only*
this configuration has no use for that header — and a file this crate did
not write may have none to give.

The key keeps every other job it has: the environment prefix is still
`APP_APP_*`, the cache entry and the diagnostics are still named after it,
and profile variants still layer on top. `Builder::new("")` is a
configuration with nothing to call itself, and then the environment layer
is the prefix alone. [Document Shape](document-shape.md) is the whole
story.

## Encrypted files

```rust
AppConfig::builder("app")
    .file("config.toml")
    .encrypted_file("secrets.json.age")
    .init()?;
```

With the `decrypt` feature (and `age` for the stock implementation), an
encrypted file decrypts through the installed
[`Decryptor`](encryption.md) and then behaves like any other layer. The
format is the extension *under* the suffix — `secrets.json.age` is JSON
that happens to be encrypted.

## Discovery

```rust
AppConfig::builder("app")
    .discover("config", ["/etc/app", "."])
    .file("local-override.toml")
    .init()?;
```

`discover(name, paths)` looks for `config.{ext}` in each directory.
Discovered files sit *below* explicitly listed ones: `file(..)` is a
deliberate statement, a search result is a guess about the machine. See
[Profiles & Discovery](profiles-and-discovery.md).

## The environment

```rust
AppConfig::builder("app")
    .file("config.toml")
    .env("APP_")
    .init()?;
```

`env("APP_")` plus the key `app` reads `APP_APP_*` — prefix, then section.
A single underscore is part of a field name; a doubled one nests:
`APP_APP_POOL__MAX_SIZE` sets `pool.max_size`. Change the separator with
`.nest("~")` when field names themselves contain doubles. The environment
merges above every file: what the deployment exports beats what the
package shipped.

## Empty variables

```rust
AppConfig::builder("app").env("APP_").allow_empty_env().init()?;
```

By default `APP_APP_HOST=` counts as *unset* — an empty export is almost
always a leftover, and "empty string" is rarely what a field wants.
`allow_empty_env()` flips that for the cases where set-to-empty is a real
value.

## Strict spellings

```rust
AppConfig::builder("app").env("APP_").strict_env().init()?;
```

figment reads environment values loosely: `8080` reaches a `u16`, `true` a
`bool`. Loose is ergonomic and ambiguous at the edges — `APP_APP_TLS=off`
reads like a boolean and arrives as the string `"off"`, silently right in
a `String` field and silently wrong everywhere else. Strict mode makes the
yes/no/on/off family (and `null`/`nil`/`none`) an error naming the
variable. Loose stays the default.

## `.env` files

```rust
AppConfig::builder("app")
    .env("APP_")
    .env_file(".env")
    .init()?;
```

With the `dotenv` feature, a `.env` file is read as the environment layer
sourced from disk — just *below* the real environment, so a variable
somebody exported for this run beats a file in the repository. Strict mode
holds `.env` files to the same spelling standard, naming the file.

## Profiles

```rust
AppConfig::builder("app")
    .file("config.toml")
    .profile_env("APP_ENV")
    .init()?;
```

When `APP_ENV=production`, every file gains a sibling layer:
`config.toml` is followed by `config.production.toml`, discovered or
listed alike. A variant that does not exist is skipped like any other
missing file.

## Runtime layers: defaults, overrides, flags

```rust
AppConfig::set_default("port", 8080)?;          // loses to everything
AppConfig::set_defaults(&AppConfig { host: "0.0.0.0".into(), port: 8080 })?;
AppConfig::set_override("host", "localhost")?;  // beats everything
AppConfig::set_assignments(["port=9999"])?;     // --set key=value strings
```

The two runtime layers bracket the rest: defaults are a fallback the
program can compute but a file need not state, overrides are what make a
test — or a `--set` flag — authoritative without touching disk. These live
on the *type* (they are its statics); the builder picks them up on every
load. With the `clap` feature, `bind_clap(&matches, &[..])` copies named
arguments into the flags layer.

## Environment bindings

```rust
AppConfig::bind_env("port", "PORT")?;
```

For the variable that already exists and is not going to be renamed:
`PORT`, `DATABASE_URL`, whatever the platform injects. A binding maps one
field to one variable by name, above the prefixed environment.

## Aliases

```rust
AppConfig::alias("pool.size", "pool.max_size")?;
```

Files written before a rename keep working: when nothing above the
defaults supplies `pool.max_size`, the value at `pool.size` fills the gap.
Chains resolve, cycles are rejected at `alias()` time.

## Loading versus installing

```rust
let candidate: AppConfig = AppConfig::builder("app").file("config.toml").load()?;

AppConfig::builder("app").file("config.toml").init()?;
let config = AppConfig::current();      // one atomic load, any thread

// The same two lines, for the one place they always pair: startup.
let config = AppConfig::builder("app").file("config.toml").init_and_current()?;
```

`load()` is a pure read — deserialize and hand over, the snapshot
untouched; use it to inspect a configuration without publishing it.
`init()` loads *and installs*, and remembers the builder so the type can
answer questions later. `init_and_current()` is `init()` with the
installed snapshot still in hand — the same install, so a reload landing
immediately afterwards moves `current()` and leaves what it returned
alone. `current()` is an atomic pointer load, cheap
enough per request — but call it once per request and reuse the `Arc`, or
a reload landing mid-request shows one request two configurations.
`try_current()` returns `None` instead of panicking;
`replace(config)` installs a value you built yourself.

`reload()` is the same install again, on demand; `reload_with(reason)` is
the same with the *reason* named, which is what a reload hook and the
`config_reload` span report — the watcher's own reloads carry
`ReloadReason::FileChanged` for exactly this.

## Validation

```rust
fn sane(config: &AppConfig) -> Result<(), dynamic_config::Error> {
    if config.port == 0 {
        return Err(dynamic_config::Error::invalid(
            "port 0 binds to a random port, which no one ever means",
        ));
    }
    Ok(())
}

AppConfig::builder("app").file("config.toml").validate(sane).init()?;
```

Deserializing successfully is not the same as being valid. The hook runs
after deserialization and before anything installs — on `init`, on every
watch reload, and on a recovery from the cache. A reload it refuses keeps
the previous snapshot serving, exactly like a parse failure.

## The last known good

```rust
use dynamic_config::CacheMode;

AppConfig::builder("app")
    .file("config.toml")
    .env("APP_")
    .cache("/var/lib/app/last.json", CacheMode::Redacted)
    .init()?;
```

Written after every clean `init` and watch reload; read when the sources
will not load, so a corrupted file does not keep a service down. The mode
is always spelled out: `Redacted` drops `#[config(secret)]` fields (they
come back from the live environment during recovery), `Full` writes
everything, `Fingerprint` writes only enough to say *what drifted* while
still refusing to start. Recovery layers the environment and `.env` files
over the cache exactly as a load would. `cache_encrypted(path, encryptor)`
writes the same cache through an [`Encryptor`](encryption.md) — full
fidelity, at rest, recovered through the installed `Decryptor` — for a
deployment that wants `Full` without a plaintext file on disk. See
[Persistence & Writing](persistence.md#last-known-good).

## A configuration with no struct

```rust
use dynamic_config::{Builder, Value};

let plugins = Builder::<Value>::values("plugins")
    .file("plugins.toml")
    .secrets(&["token"])
    .load()?;

plugins.get("cache.ttl").and_then(Value::as_u64);
```

`Builder::values(key)` is sugar for `Builder::<Value>::new(key)`, for the
keys a program learns at run time rather than declares — a plugin host, a
feature-flag table, a tool reading somebody else's file. Every layer,
profile, watcher, cache and diagnostic works unchanged, because nothing in
the engine ever knew what `T` was.

Two things a struct declares are gone, and both are *reported* rather than
assumed: there is no field list, so `check()` says
`unknown keys: not checked (no field list)` instead of an empty
all-clear — and nothing is marked secret, so `.secrets(&[..])` is how a
schemaless configuration says what to redact. Without it a redacting cache
mode is refused rather than written unredacted. See
[Schemaless Configuration](schemaless.md).

## Hot reload

```rust
let builder = AppConfig::builder("app").file("config.toml");
builder.init()?;

let _watch = builder.watch(Duration::from_millis(250))?;   // keep the handle
```

Edits reload in the background: directory-watched (editors rename, they
do not write in place), debounced, validated, and swapped atomically — a
bad edit degrades to "no change" plus a report. Dropping the handle stops
the watch; `.detach()` keeps it for the life of the process. One watcher
per type, whoever starts it: a second `watch()` is `AlreadyExists`. For
filesystems where notification silently never fires — NFS, some bind
mounts — choose polling explicitly:

```rust
use dynamic_config::watch::WatchMode;

let _watch = builder.watch_with(
    Duration::from_millis(250),
    WatchMode::Poll { interval: Duration::from_secs(2) },
)?;
```

## Callbacks

```rust
AppConfig::on_reload(|old, new| {
    if old.port != new.port {
        tracing::warn!("port changed; takes effect on restart");
    }
});

let guard = AppConfig::on_reload_scoped(|_, new| metrics.set_port(new.port));
drop(guard);    // unregistered from here on
```

`on_reload` is permanent — right for process-lifetime wiring;
`on_reload_scoped` lasts until its `HookGuard` drops. Callbacks run on
whichever thread performed the reload, so keep them short: compare, then
*signal* the subsystem that owns the resource. A panicking callback is
caught and logged; the rest still run. The boundary — reloading
configuration reloads nothing *built from* it — has
[its own chapter](reload-lifecycle.md).

## Awaiting changes

```rust
let mut reloads = AppConfig::changes();

loop {
    let config = reloads.changed().await;
    pool.resize(config.port as usize).await;
}
```

The awaitable form of the same event, with the `async` feature — a plain
`Future`, so tokio, smol, Embassy and a hand-written executor all drive
it. A handle created *before* anything installs sees the first install as
its first change: `changes()` doubles as "wake me when configuration
exists", by contract.

## Async loading

```rust
let builder = AppConfig::builder("app").file("config.toml");
builder.init_async().await?;
let candidate = builder.load_async().await?;
```

Reading files is blocking work; these hand it to a blocking worker instead
of stalling an executor thread. By default that worker is a fresh thread —
reloads are rare — and the `tokio` feature routes it to the blocking pool;
`set_blocking_executor` installs anything else.

## Grouped reloads

```rust
use dynamic_config::ReloadGroup;

let group = ReloadGroup::new().with::<AppConfig>().with::<TlsConfig>();
group.reload()?;
```

When two types must move together — a certificate path and the port it is
served on — a group loads and validates *every* member before installing
*any*, so a failure leaves all of them on their previous snapshots. Each
member answers through the builder its `init()` remembered.

`builder.prepare()` is the piece that makes that possible, and it is
public: it loads and validates and hands back a `Commit` that installs
when called, so a caller can stage several configurations and decide
afterwards whether any of them lands.

## Remote stores

```rust
AppConfig::set_remote(EtcdSource::new(client, "/app/config"));
AppConfig::refresh_remote()?;                 // the network round trip
AppConfig::builder("app").env("APP_").init()?; // store + env, one configuration
```

A store layers above the files and below the environment. `current()`
never touches the network: `refresh_remote()` (or `refresh_remote_async`)
is the explicit round trip, and a store's watch pushes documents into
the sink from `remote_sink()`, which reloads through the remembered
builder — validation, hooks, cache and all. Eight store crates ship — seven
over a network, and git; [Remote Stores](remote-stores.md) has the contract.

## Diagnostics

```rust
let report = builder.check()?;                    // would it load? any unknown keys?
let origin = AppConfig::source_of("port")?;       // which layer wins next load
let now = AppConfig::snapshot()?.source_of("port").cloned(); // and in this snapshot
println!("{}", AppConfig::explain("port")?);      // every layer's answer, as a table
```

`check` reports without loading and names unknown keys; `is_set(path)` is
the one-question form of the same, answering whether anything supplies a
path at all. `source_of` answers for the *next* load,
`snapshot().source_of` for the resolved snapshot in hand. `explain` shows the whole argument — every layer's
value and the winner — and is the one diagnostic that prints values;
`#[config(secret)]` fields stay `***` in it. All four work on the builder
before any `init`, and on the type after one. See
[Validation & Diagnostics](validation-diagnostics.md).

## Schema

```rust
let schema = AppConfig::builder("app").schema();
std::fs::write("config.schema.json", schema.to_string())?;
```

With the `schema` feature and `derive(JsonSchema)`: a JSON Schema for the
*file* the section lives in — the struct's schema wrapped under its key,
secrets marked `writeOnly`. `schema::merge` combines several types that
share one file, which is what gives editors validation and completion.

## Saving

```rust
dynamic_config::save(&config, "config.json", Format::Json, "app")?;
dynamic_config::save_new(&config, "config.json", Format::Json, "app")?;   // refuses to overwrite
```

Writing back is a free function — it needs `Serialize` and nothing from
the type's storage. Writes are atomic and fsynced: temp file, flush,
rename, directory sync. `save_encrypted` does the same through an
[`Encryptor`](encryption.md).

## Observability

```rust
AppConfig::on_reload(|old, new| {
    for change in dynamic_config::changed_paths(old, new).unwrap_or_default() {
        tracing::info!(target: "audit", %change, "configuration changed");
    }
});
```

With the `tracing` feature every watcher reload is a `config_reload` span
carrying outcome and duration — enough to alert on "has not reloaded
cleanly in an hour". `changed_paths` names what moved, paths only, never
values: the audit half of a reload, holding the same line every other
diagnostic in this crate holds.

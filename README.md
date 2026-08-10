# dynamic-config

Hot-reloadable, lock-free application configuration for Rust, behind one attribute.
Built on [figment].

```rust
use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(
    files = ["config.toml", "secrets.json"],
    key   = "db",
    env   = "APP_",
    watch,
)]
#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    DatabaseConfig::init()?;        // load once, fail fast on a bad config
    DatabaseConfig::start_watch()?; // reload in the background from now on

    let config = DatabaseConfig::current();
    println!("{}:{}", config.host, config.port);

    Ok(())
}
```

```toml
[dependencies]
dynamic-config = { version = "0.0.1", features = ["toml", "watch"] }
```

## Features

Every one of these is described in full further down; this is the map.

### Loading

| | |
|---|---|
| **Formats** | JSON, TOML, YAML — each behind its own feature, and using one that is off is a compile error naming it |
| **Several files, merged** | `files = ["config.toml", "secrets.json"]`, left to right; a file that is not there is skipped, which is what makes an optional `secrets.json` work |
| **Discovery** | `name = "config"` with `paths = ["/etc/myapp", "~/.config/myapp", "."]`; `~` expands, and resolution happens per load so a file that appears later is picked up |
| **Profiles** | `profile_env = "APP_ENV"` layers `config.production.toml` over `config.toml`, for discovered and listed files alike |
| **Encrypted files** | `secrets.json.age` decrypts at load time; the suffix marks it, the extension under it names the format |
| **`.env` files** | `env_files = [".env"]`, read as the environment layer rather than as documents — and without touching the process environment |
| **Any figment provider** | `Source::provider(..)` behind the `figment` feature, for `Serialized::defaults(T)`, a custom `Env`, or one you wrote |
| **No files at all** | `files = []` for a container fed by a store and the environment |

### Layers

```text
defaults < files < remote < .env < APP_DB_* < bind_env < flags < overrides
```

| | |
|---|---|
| **Environment** | `env = "APP_"` with configurable nesting (`APP_DB_POOL__MAX_SIZE`), and `FOO=` treated as unset unless you say otherwise |
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
| **Poll fallback** | `poll` / `poll_interval` for NFS and overlay filesystems, where inotify registers and then silently delivers nothing |
| **Debounce** | one editor save is several filesystem events |
| **Remote stores** | etcd, Consul, NATS, Redis, Vault, S3 and Firestore — each watching the way its protocol allows |
| **Hooks** | `on_reload(previous, current)`, and `changes()` for a task that would rather await |
| **Any runtime, or none** | `changes()` is a `Future` over a generation counter and a list of wakers; [tokio](dynamic-config/examples/tokio_runtime.rs), [smol](dynamic-config/examples/smol_runtime.rs) and [Embassy](dynamic-config/examples/embassy_runtime.rs) all drive it |
| **All-or-nothing** | `ReloadGroup` prepares every member before any of them commits |
| **Key-level diffs** | `diff` logs which keys moved — paths only, never values |

### Safety

| | |
|---|---|
| **Validation** | `validate` runs your own check on every load; a reload that fails it keeps the previous snapshot |
| **A bad reload cannot take the process down** | the running snapshot stays until a new one is complete and valid |
| **Secret redaction** | `#[config(secret)]` prints `***`, and `#[derive(Debug)]` alongside it is a compile error rather than a race between two impls |
| **Nothing leaks a value** | diffs, `check()` reports, unknown-key suggestions and error messages all report paths and types, never values |
| **Files written are private** | `save` and the cache *create* their file `0600` and refuse to follow a symlink planted at the temporary path |
| **Writing without replacing** | `save_new` refuses if the file exists, for a setup wizard that must not overwrite what somebody wrote |
| **Writing encrypted** | `save_encrypted` to a recipient list, the counterpart to reading a `secrets.json.age` |
| **Last known good** | `cache` starts from yesterday's configuration when today's is broken, in three modes so what lands on disk is a choice |

### Diagnostics

| | |
|---|---|
| **Provenance in every error** | `pool.max_size: invalid type: found a string, expected u16 (from APP_DB_)` |
| **`source_of` / `is_set`** | which layer supplies a key, and whether anything does |
| **`check()`** | what the configuration resolves to, without loading it — works when the load fails, which is when it is worth running |
| **Unknown keys** | with suggestions from a transposition-aware edit distance, so `prot` finds `port` |
| **A JSON Schema** | `schema()` describes the *file*, marks secrets `writeOnly`, and drops `required` because a file is one layer of six |

### The shape of the crate

| | |
|---|---|
| **Three mandatory dependencies** | `figment`, `serde`, `arc-swap`. Every format, client, crypto stack and runtime is behind a feature or in a companion crate |
| **`#![forbid(unsafe_code)]`** | in every crate here, checked by CI rather than trusted |
| **MSRV 1.71** | and every feature that raises it says so, verified against real toolchains |
| **No global singleton** | each configuration type owns its storage; there is no `Config::get()` returning something a library set |
| **`no_std`** | a separate crate for microcontrollers: no filesystem, no allocator, no runtime |

## Why

Configuration in a long-running service has three awkward properties at once:
it comes from several sources with a precedence order, it is read on nearly
every request from many threads, and it should be changeable without a restart.

Doing that by hand means a `RwLock<Config>` on the read path, a bespoke file
watcher, and a reload that must not take the process down when someone saves a
broken file. This crate is all three.

|  | [`config`] | [`figment`] | Go's [Viper] | **dynamic-config** |
|---|---|---|---|---|
| Layered sources | ✅ | ✅ | ✅ | ✅ |
| Hot reload | ❌ | ❌ | ✅ | ✅ |
| Lock-free reads | — | — | **not thread-safe** | ✅ |
| Reload keeps last good config | — | — | ❌ | ✅ |
| Typed struct API | ✅ | ✅ | partial | ✅ |
| Async: await config changes | ❌ | ❌ | callback | ✅ |

The loader is figment — layered providers, profile selection and loose typing of
environment values are problems it already solves well. What this crate adds is
everything around it: the attribute, the lock-free snapshot, the watcher, and a
reload that cannot take the process down.

## Attribute reference

Every argument `#[dynamic_config(..)]` accepts, and every field attribute.

### At a glance

| Argument | Form | Requires | Default |
|---|---|---|---|
| [`files`](#files) | `files = ["a.toml", "b.json"]` | one of `files` / `name`+`paths` | — |
| | `files = []` | | no files at all, on purpose |
| [`name`](#name--paths) | `name = "config"` | `paths` | — |
| [`paths`](#name--paths) | `paths = ["/etc/app", "."]` | `name` | — |
| [`key`](#key) | `key = "db"` | always required | — |
| [`env`](#env) | `env = "APP_"` | | no environment layer |
| [`nest`](#nest) | `nest = "__"` | `env` | `"__"` |
| [`allow_empty_env`](#allow_empty_env) | flag | `env` | off — `FOO=` is unset |
| [`profile_env`](#profile_env) | `profile_env = "APP_ENV"` | | no profile overlay |
| [`watch`](#watch) | flag | `watch` feature | off |
| [`debounce`](#debounce) | `debounce = 250` | `watch` | `250` ms |
| [`poll`](#poll--poll_interval) | flag | `watch` | native backend |
| [`poll_interval`](#poll--poll_interval) | `poll_interval = 2000` | `watch` | `2000` ms with `poll` |
| [`diff`](#diff) | flag | | off |
| [`validate`](#validate) | flag | a `validate()` on the type | off |
| [`save`](#save) | flag | `Self: Serialize` | off |
| [`cache`](#cache) | `cache = "/var/lib/app/last.json"` | | no cache — a bad start fails |
| [`cache_mode`](#cache_mode) | `cache_mode = "redacted"` | `cache` | `"full"` |
| [`env_files`](#env_files) | `env_files = [".env"]` | `dotenv` feature + `env` | none |
| [`schema`](#schema) | flag | `schema` feature + `Self: JsonSchema` | off |
| [`async`](#async) | flag | `async` feature | off |

| Field attribute | Form | Effect |
|---|---|---|
| [`secret`](#configsecret) | `#[config(secret)]` | `Debug` prints `***`; forbids `#[derive(Debug)]` |

Anything else is a compile error listing the arguments that exist.

---

### `files`

```rust
#[dynamic_config(files = ["config.toml", "secrets.json"], key = "db")]
```

Sources merged **left to right** — later files win. The format comes from the
extension (`.json`, `.toml`, `.yaml`, `.yml`); using one whose feature is off is
a compile error naming the feature to add. A file that does not exist is
skipped, which is what makes an optional `secrets.json` work.

Paths resolve against the working directory. For a deployment, prefer
[`name` + `paths`](#name--paths).

Either `files` or `name` + `paths` is required. Both together is fine: the
explicitly listed files win, because a listed file is a deliberate statement and
a search result is a guess about the machine.

A `.age` suffix marks a file as [encrypted](#encrypted-config-files):
`secrets.json.age` is JSON that happens to be ciphertext.

`files = []` says **no files, on purpose** — the shape of a container whose
configuration comes from a [remote store](#remote-sources) and the environment
alone. Omitting `files` entirely is still an error, because that is a mistake
rather than a decision.

### `name` + `paths`

```rust
#[dynamic_config(
    name  = "config",
    paths = ["/etc/myapp", "~/.config/myapp", "."],
    key   = "db",
)]
```

Looks for `{name}.{ext}` in each directory, in order. **Every** directory with a
match contributes one file, layered in search order — so `/etc` defaults,
`~/.config` overrides and a local `./config.toml` all apply, in that order.
(Go's Viper stops at the first hit; the reason to list `/etc` *and* `~` is to
layer them.)

Within one directory the extensions are tried `.toml`, `.json`, `.yaml`, `.yml`,
skipping any whose feature is off, and the first hit wins — so a stray
`config.json` next to a `config.toml` resolves the same way every run.

`~` expands via `HOME`, or `USERPROFILE` on Windows. Resolution happens per
load, so a file that appears later is picked up by the next reload rather than
requiring a restart.

Neither half works alone: `name` without `paths` would search nowhere, `paths`
without `name` would search for nothing. Both are compile errors.

### `key`

```rust
#[dynamic_config(files = ["config.toml"], key = "db")]
```

The section this struct maps to. Every file's **top-level** keys are sections,
so several config types can share one file:

```toml
[db]      # -> DatabaseConfig
host = "localhost"

[server]  # -> ServerConfig
port = 8080
```

A consequence worth knowing: every top-level key must be a table. A stray
`"_comment": "..."` at the top level is a parse error, not an ignored key.

### `env`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", env = "APP_")]
```

Combined with `key`, so `env = "APP_"` and `key = "db"` read `APP_DB_*`. The
environment is merged after every file and wins over all of them.

| Variable | Sets |
|---|---|
| `APP_DB_HOST` | `host` |
| `APP_DB_MAX_SIZE` | `max_size` |
| `APP_DB_POOL__MAX_SIZE` | `pool.max_size` |

Values are read loosely: `8080` reaches a `u16`, `true` a `bool`, `[a, b, c]` a
`Vec<String>`. A value that cannot become the field's type is an error naming
the field.

### `nest`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", env = "APP_", nest = "___")]
```

The separator that introduces nesting in a variable name. Defaults to `__`.

A single separator cannot mean both "word break" and "nesting" — that is why the
default is doubled — so whatever this is set to must be something a field name
will not contain. Requires `env`.

### `allow_empty_env`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", env = "APP_", allow_empty_env)]
```

By default `APP_DB_HOST=` counts as **unset** and the file's value survives. An
unset value rendered into a deployment template leaves exactly `FOO=`, and
letting that blank out a good configured value is a bad afternoon.

Turn this on when empty really is a value you need to be able to send. Requires
`env`.

### `profile_env`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", profile_env = "APP_ENV")]
```

Names the variable holding the active profile. With `APP_ENV=production`, every
file gains a sibling layered over it: `config.toml`, then
`config.production.toml`. Works for discovered files too; a variant that does
not exist is skipped like any other missing file.

The profile is read at load time, so it follows the environment rather than the
build.

**A variant sits directly on top of its own base**, not above the search order:

```text
/etc/myapp/config.toml
/etc/myapp/config.production.toml
~/.config/myapp/config.toml            ← still wins over the line above it
~/.config/myapp/config.production.toml
```

So a later directory's plain file beats an earlier directory's variant. That is
the search order doing its job — a user's file is more specific to the machine
than a package's production defaults — but it is worth knowing before relying on
the opposite.

### `watch`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch)]
```

Generates `start_watch()`, which reloads the snapshot when a file changes.
Requires the `watch` feature.

**The returned handle owns the watcher** — dropping it stops watching:

```rust
Config::start_watch()?.detach();       // a server: watch for the whole process
let _watch = Config::start_watch()?;   // a test, a subcommand: stop with the scope
```

Directories are watched rather than files, because editors and `mv`-based atomic
saves replace the inode. Kubernetes ConfigMap updates arrive as a `..data`
symlink swap and are recognised as changes.

A reload that fails is logged and the previous snapshot is kept.

### `debounce`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, debounce = 500)]
```

Quiet period in milliseconds before a reload fires. One editor save typically
emits several filesystem events; waiting collapses them into one reload. Must be
non-zero. Requires `watch`.

### `poll` / `poll_interval`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, poll_interval = 2000)]
```

Detect changes by re-reading on an interval instead of by notification. `poll`
alone uses 2000 ms.

Needed because inotify and its equivalents do not fire on many network and
overlay filesystems — NFS, some Docker bind mounts, some CI runners. The failure
is **silent**: the watch registers and simply never delivers, so there is nothing
to detect and fall back from. It has to be chosen deliberately. Requires `watch`.

### `diff`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, diff)]
```

```text
[dynamic-config] DbConfig: reloaded, pool.max_size changed, tls added
```

Logs which keys a reload changed. **Paths only, never values** — otherwise a
reload of `db.password` would do in the log exactly what `#[config(secret)]`
exists to prevent. Costs no extra file reads: the reload resolves once and both
deserializes and compares.

Applies to every reload, not only the watcher's: a document a
[remote watch](#watching-a-store) pushed through `apply_remote` is reported the
same way. That is why it needs no `watch` — a program with no config file at all,
watching only a store, still wants to know what moved.

### `validate`

```rust
#[dynamic_config(files = ["config.toml"], key = "pool", validate)]
#[derive(Deserialize, Validate)]        // validator, garde, or a method of your own
struct Pool { min_size: u16, max_size: u16 }
```

Every load calls `self.validate()` and turns an `Err` into `ErrorKind::Invalid`,
so a reload that fails validation keeps the previous snapshot exactly as a parse
failure does. For the case where every field is valid on its own and the whole
is still nonsense.

`validate` is resolved at **your** call site — an inherent method, or any trait
in scope — so this crate never pins a version of a validation library.

### `save`

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

### `cache`

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

### `cache_mode`

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

### `env_files`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", env = "APP_", env_files = [".env"])]
```

`.env` files, merged in order just below the real environment. Requires the
`dotenv` feature and an `env` prefix — a `.env` holds variable names, and
without a prefix there is no rule for which of them belong to this section. See
[`.env` files](#env-files).

### `schema`

```rust
#[dynamic_config(files = ["config.json"], key = "db", schema)]
#[derive(Deserialize, JsonSchema)]
```

Generates `schema()`. Requires the `schema` feature and `Self: JsonSchema` —
opt-in for the same reason `save` is: the method needs a trait you have to
derive, and a `where Self: JsonSchema` clause cannot express that (rustc rejects
an inherent method whose bound a concrete `Self` does not meet, at the
definition rather than at the call). See
[A schema for the config files](#a-schema-for-the-config-files).

### `async`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, async)]
```

Generates `load_async()`, `init_async()` and `changes()`. Requires the `async`
feature — which pulls in **no runtime at all**. See [Async](#async-1).

### `#[config(secret)]`

```rust
#[dynamic_config(files = ["config.toml"], key = "db")]
#[derive(Deserialize)]          // note: no `Debug`
struct DatabaseConfig {
    host: String,
    #[config(secret)]
    password: String,
}
// DatabaseConfig { host: "localhost", password: "***" }
```

Generates a `Debug` that redacts the marked fields. `#[derive(Debug)]` alongside
it is a compile error rather than a race between two impls.

---

## What the attribute generates

| Method | Always | Description |
|---|---|---|
| `load() -> Result<Self, Error>` | ✅ | Read the sources and deserialize. Leaves the snapshot alone. |
| `init() -> Result<(), Error>` | ✅ | `load()` plus install as the initial snapshot. |
| `replace(Self)` | ✅ | Atomically swap in a new snapshot. |
| `current() -> Arc<Self>` | ✅ | The current snapshot. Panics before `init()`. |
| `try_current() -> Option<Arc<Self>>` | ✅ | The current snapshot, or `None`. |
| `snapshot() -> Result<Snapshot, Error>` | ✅ | Resolve without deserializing, for keys with no field. |
| `source_of(path)` / `is_set(path)` | ✅ | Which layer supplies a key, and whether anything does. |
| `check() -> Result<Report, Error>` | ✅ | What it resolves to, without loading. |
| `on_reload(f)` | ✅ | Run a callback on every later reload. |
| `prepare() -> Result<Commit, Error>` | ✅ | Load without installing, for a `ReloadGroup`. |
| `set_default` / `set_override` / `set_flag` | ✅ | The three runtime layers. |
| `set_assignments(["k=v"])` | ✅ | Apply `--set key=value` strings. |
| `bind_env(path, "PORT")` / `clear_env_bindings()` | ✅ | Bind a field to an environment variable by name. |
| `clear_defaults` / `clear_overrides` / `clear_flags` | ✅ | Drop them again. |
| `start_watch() -> io::Result<WatchHandle>` | `watch` | Reload on file changes. Idempotent. |
| `save(&self, path)` | `save` | Write back, atomically. |
| `save_new(&self, path)` | `save` | The same, refusing if the file exists. |
| `save_encrypted(&self, path, &encryptor)` | `save` + `decrypt` | The same, encrypted. |
| `alias(from, to)` / `clear_aliases()` | ✅ | Keep an old key path working after a rename. |
| `load_async()` / `init_async()` | `async` | The same, off the async executor. |
| `changes()` | `async` | A handle woken by every later reload. |
| `set_remote(source)` / `refresh_remote()` / `clear_remote()` | ✅ | Install a remote store, fetch from it, drop what it gave. |
| `apply_remote(document)` | ✅ | Install a document a watch pushed, and reload. |
| `set_remote_async(source)` / `refresh_remote_async()` | `async` | The same, for a store whose client is async. |
| `bind_clap(&matches, &[..])` | `clap` feature | Copy arguments into the flags layer. |
| `schema()` | `schema` argument | A JSON Schema for the file this section lives in. |

## Precedence

```text
set_default < discovered < config.toml < secrets.json < remote < APP_DB_* < bind_env < set_flag < set_override
 (runtime)   (search path)   (first)      (last file)   (etcd…) (environment) (by name)  (CLI)     (runtime)
```

The two runtime layers bracket the rest:

```rust
DbConfig::set_default("pool.max_size", num_cpus::get() * 4)?;  // a computed fallback
DbConfig::set_override("host", "localhost")?;                  // a test, or --set
DbConfig::clear_overrides();
```

Defaults cover a fallback the program can compute but a file need not state —
`#[serde(default)]` handles the constant case, this handles the case where the
value is only known at run time. Overrides win over everything, which is what
makes them useful in tests and behind a `--set key=value` flag. Both take effect
on the next `load()`, and an error in either says `set as override` rather than
blaming a file.

Tables merge key by key, so a three-line `secrets.json` can override two fields
of a large `config.toml` without restating the rest. Arrays are replaced
wholesale, never concatenated — there is no reading of `["a"] + ["b"]` that is
right for every caller, and a silent append cannot be undone by a later file.

## Remote sources

Configuration served from somewhere other than this machine — etcd, Consul,
NATS, Vault — arrives as a document and merges like a file, above the files and
below the environment.

| Crate | Store | Trait | Reads | Watches by | Authenticates with |
|---|---|---|---|---|---|
| [`dynamic-config-etcd`](dynamic-config-etcd) | etcd v3 | async | one key, a whole document | a watch stream | user/password, TLS |
| [`dynamic-config-consul`](dynamic-config-consul) | Consul KV | blocking | one key, a whole document | a blocking query | ACL token, Kubernetes, JWT/OIDC |
| [`dynamic-config-nats`](dynamic-config-nats) | NATS JetStream KV | async | one key, a whole document | a KV change stream | token, user/password, NKey, JWT, creds |
| [`dynamic-config-redis`](dynamic-config-redis) | Redis | blocking | one key, a whole document | keyspace notifications | in the URL, TLS |
| [`dynamic-config-vault`](dynamic-config-vault) | Vault KV v2 | blocking | one path, a map of fields | polling the version | token, AppRole, Kubernetes, JWT/OIDC, userpass, LDAP, cert |
| [`dynamic-config-s3`](dynamic-config-s3) | S3, and anything speaking it | async | one object, a whole document | polling the ETag | the AWS credential chain |
| [`dynamic-config-firestore`](dynamic-config-firestore) | Firestore | blocking | one document, a map of fields | polling `updateTime` | workload identity, an access token |

Each has its own README with the whole story, and an example that runs against a
real server in a container.

Each is a separate crate so that reaching for one store does not put the
others' dependency trees — a gRPC stack, a streaming client, the AWS SDK,
several HTTP clients — into a build that never asked for them.

```rust
DbConfig::set_remote(Consul::new("http://consul:8500", "myapp/db.json")?);

DbConfig::refresh_remote()?;   // the network round trip, explicitly
DbConfig::init()?;             // merges what came back; touches no network
```

### Fetching is explicit

A remote source is **not** read on every `load()`. Configuration is read on
nearly every request, so a network round trip there would be indefensible — and
it is also what would force every async question to become a blocking one.

```text
refresh_remote()   →  fetch, keep the document
load()             →  merge the kept document, no I/O
```

That one decision is what lets a blocking source and an async source sit side by
side with no `block_on` anywhere, on any runtime or none. Pair it with whatever
already schedules work in your program — a timer, a signal handler, a watch
stream.

### Two traits, because two kinds of client exist

```rust
pub trait RemoteSource: Send + Sync + 'static {
    fn fetch(&self) -> Result<Fetched, Error>;
    fn describe(&self) -> String;
}

#[cfg(feature = "async")]
pub trait AsyncRemoteSource: Send + Sync + 'static {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = Result<Fetched, Error>> + Send + '_>>;
    fn describe(&self) -> String;
}
```

Consul and Vault have plain HTTP APIs, so implementing the blocking trait costs
their users no runtime. etcd speaks gRPC and NATS is a streaming protocol, so
both of those clients are async to begin with and pretending otherwise would
just hide a `block_on`.

`refresh_remote_async()` accepts **either**, running a blocking source inline —
so swapping one implementation for the other is not a breaking change for the
caller. `refresh_remote()` refuses an async source and says which call to use
instead, rather than reaching for a runtime it was never given.

### Watching a store

Polling on a timer works, and is what Vault, S3 and Firestore have to do — but
etcd, NATS, Consul and Redis can say the moment a value moves. Each companion
crate owns that loop, because a
watch is long-lived and protocol-shaped in a way one trait cannot honestly
cover; what they all push through is `apply_remote`:

```rust
// etcd, NATS and S3: a future. Cancelled by dropping it, on any executor.
tokio::spawn(async move { etcd.watch(DbConfig::apply_remote).await });

// Consul, Vault, Redis and Firestore: a thread, so it takes a stop token.
let watch = RemoteWatch::new();
let watching = watch.watching();

std::thread::spawn(move || consul.watch(&watching, DbConfig::apply_remote));
```

`apply_remote` is the sink, and it is the *same reload path a file edit takes* —
validation, the reload hooks, the diff, the cache. A document that does not fit
leaves the previous snapshot serving and returns the error, exactly as a bad
file edit does.

Three things behave the same way across all seven, because they are decisions
rather than accidents:

- **The current value is not delivered at startup.** A watch reports changes;
  announcing the value the caller already has would make every restart look like
  an edit. Fetch first if the starting value matters — it usually does.
- **A deleted key is not a change.** No configuration is not a configuration, and
  neither replaying the last one nor pushing emptiness is better than leaving the
  running snapshot alone.
- **A transport failure does not end the watch.** The store restarting is
  precisely what a watch is there to survive; the loop backs off and retries.
  Only an error from *your* callback ends it, so a caller that wants to survive a
  bad document should log it and return `Ok`.

Cancellation splits along the same line the traits do. An async watch is a
future: drop it. A blocking watch is a thread, which cannot be dropped from
outside, so it takes a [`Watching`] token and checks it between requests —
dropping the matching `RemoteWatch` stops it, the same contract `WatchHandle`
has for files.

How long stopping takes is the one thing worth knowing per store:

| Crate | Worst case for noticing a stop |
|---|---|
| etcd, NATS | immediate — the future is cancelled |
| Consul | the blocking query's `wait`, one minute by default |
| Vault, Redis, S3, Firestore | a quarter second, whatever the poll interval is |

### Credentials, and keeping them working

Every store has its own way in, and every one of them expires. Three rules hold
across all seven crates:

**Logging in is lazy.** Building a source reaches nothing; the first read does
it. Constructing a source is not I/O, and configuration that hits the network on
a call nobody expected to block is how a startup ends up mysteriously slow.

**Expiry is handled on both sides.** A credential close to its expiry is renewed
or replaced *before* the request; one that turns out to be dead is replaced
*after* it, and the request retried — once. Clocks skew and tokens get revoked,
so the proactive path cannot catch everything; and a second refusal means the
policy is wrong, so retrying again would turn a clear failure into a hang.

**A credential read from a file is re-read at every login.** Kubernetes rotates
projected service-account tokens, and a copy taken at startup expires with the
pod still running.

Each crate speaks its store's own vocabulary rather than inventing one: etcd and
NATS take their own `ConnectOptions` (re-exported, so no direct dependency),
while Vault and Consul get an `Auth` enum because their login endpoints have no
equivalent type.

### Sharing a client you already have

```rust
Etcd::from_client(client, "myapp/db.json")          // etcd
Nats::from_client(client, "config", "db.json")      // NATS
Consul::new(address, key).with_agent(agent)         // Consul
Vault::new(address, mount, path).with_agent(agent)  // Vault
```

For a program that already talks to the store, or one with its own proxy
settings, private CA, client certificate or connection pool. A shared client is
not a second-class one: it recovers from an expired credential like any other,
because the credentials live in the client rather than in the source.

### Writing your own

Implement one trait, return the document and its format:

```rust
impl RemoteSource for MyStore {
    fn fetch(&self) -> Result<Fetched, Error> {
        let text = self.http_get("/config")?;

        Ok(Fetched::new(text, Format::Json))
    }

    fn describe(&self) -> String {
        format!("my-store {}", self.address)   // this lands in error messages
    }
}
```

A failed fetch leaves the previously fetched document in place, so an
unreachable store does not take a working process down with it.

## Reading is lock-free

The snapshot lives in a `OnceLock<ArcSwap<T>>`. `current()` clones an `Arc` out
of it, so a reload never blocks a request handler, and a reader that already
holds an `Arc` keeps its own generation.

**Call `current()` once per unit of work** and reuse the `Arc`. Calling it twice
inside one request can straddle a reload and observe two configurations.

## Reloading cannot take the process down

A reload re-runs `load()`. If the new configuration is invalid, or a file is
caught half-written, the error is reported and the previous snapshot stays in
place. A bad edit degrades to "no change".

**`start_watch()` returns a handle, and dropping it stops the watcher.** A
server calls `.detach()` to watch for the rest of the process; anything with a
lifecycle — a test, a library, a subcommand — binds the handle so watching stops
when the thing being configured goes away.

```rust
Config::start_watch()?.detach();       // a server
let _watch = Config::start_watch()?;   // a test, a subcommand
```

The watcher observes the **directory** holding each file rather than the file
itself: editors and `mv`-based atomic saves replace the inode, which silently
detaches a file-level watch. That is also what makes a Kubernetes ConfigMap
update — delivered as a `..data` symlink swap — visible at all.

## Encrypted config files

A `secrets.json` in a repository is a problem everyone recognises. Encrypt it
with [`age`](https://age-encryption.org) and it decrypts at load time:

```text
config.toml            plain, in the repository
secrets.json.age       ciphertext, in the repository
```

```rust
// Once, before anything loads. A key is a process-wide fact, so this is a
// process-wide setting.
dynamic_config::set_decryptor(dynamic_config::age::Age::from_environment()?)?;

#[dynamic_config(files = ["config.toml", "secrets.json.age"], key = "db")]
#[derive(Deserialize)]
struct DbConfig {
    host: String,
    #[config(secret)]
    password: String,
}
```

The `.age` suffix marks the file as encrypted; the extension **under** it says
what the plaintext is, so `secrets.json.age` is JSON. Everything else is
unchanged — same precedence, same profile variants
(`secrets.production.json.age`), watched the same way, skipped if it is not
there, and a value traced back to it names the file rather than "an inline
source".

The key comes from `SOPS_AGE_KEY_FILE`, `AGE_IDENTITY_FILE` or `AGE_SECRET_KEY`,
in that order — the SOPS variable first, because a machine set up for SOPS
already has it. `Age::from_identity_file`, `Age::from_key` and
`Age::from_passphrase` name one explicitly. Both binary and armored files are
read without being told which.

A file this key cannot open is an error naming the file, not a file quietly
skipped: a configuration that silently lost its secrets is worse than one that
refuses to start.

### What it does not do

**It does not keep secrets out of memory.** The resolved configuration holds
every value, because that is what configuration *is* — a program that can use a
password can read it. The decrypted text is zeroized once parsed, and
`#[config(secret)]` keeps values out of logs, but neither is a claim about
process memory.

**The cache is still plaintext.** [`save`](#save) has an encrypting counterpart
— `save_encrypted`, taking the recipients at the call site, because *who may
read this file* is a decision about that write rather than a property of the
process. The [last-known-good cache](#last-known-good) writes plaintext and says
so.

**It is not SOPS.** SOPS encrypts values in place and verifies a MAC over the
document — a format worth implementing properly or not at all. What is here
instead is the `Decryptor` trait: implement it, install it, and any scheme
works, including shelling out to `sops -d`.

```rust
impl Decryptor for MyScheme {
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, Error> { .. }
    fn describe(&self) -> String { "my-kms".to_owned() }
}
```

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

## Generic configuration types

`Config<Postgres>` and `Config<Mysql>` are different types, so they get
different snapshots:

```rust
#[dynamic_config(files = ["config.toml"], key = "db")]
#[derive(Debug, Deserialize)]
struct Db<D: Driver> {
    url: String,
    #[serde(skip)]
    driver: PhantomData<fn() -> D>,   // `fn() -> D`, so the marker stays Send + Sync
}

Db::<Postgres>::init()?;
Db::<Mysql>::init()?;                 // its own snapshot, its own layers
```

Type and const parameters both work. A **lifetime** parameter does not, and is
rejected at compile time: the snapshot outlives every borrow that could name
one.

### It is not free, so you only pay for it if you use it

Rust has no generic statics, so a generic type's snapshot cannot live in one. It
goes through a `TypeId`-keyed registry instead. Measured on this machine with
`cargo bench -p dynamic-config --features json`, 5M reads each:

| Storage | `current()` |
|---|---|
| `static ConfigCell` (non-generic) | **17 ns** |
| `TypeId` registry (generic) | **27 ns** |

The macro knows which shape it is emitting, so a non-generic config type keeps
its `static` and its 17 ns — adding generic support cost existing code nothing.
The registry read is lock-free (an `ArcSwap` of the table, and `TypeId` passed
through rather than hashed); the first naive version, with an `RwLock` and
SipHash, measured 64 ns.

Either figure is noise next to a request. Both are the cost of *taking* a
snapshot, not of reading fields from one — take it once per unit of work and the
question stops mattering.

## Units

`timeout = 30` is ambiguous and `max_body = 67108864` is unreadable, so both are
usually written with a unit — which no stock `Deserialize` accepts:

```rust
#[derive(Deserialize)]
struct Limits {
    #[serde(with = "dynamic_config::duration")]
    timeout: Duration,      // "30s", "1h30m", "500ms", or a number of seconds
    #[serde(default, with = "dynamic_config::duration::option")]
    grace: Option<Duration>,
    #[serde(with = "dynamic_config::bytes")]
    max_body: u64,          // "64MiB", "1GB", or a number of bytes
}
```

`KiB`/`MiB`/`GiB` are powers of 1024, `KB`/`MB`/`GB` powers of 1000, and a bare
`K`/`M`/`G` is read as the binary form. An unknown unit is an error listing the
valid ones, never a silent zero.

## Async

The `async` feature brings in **no runtime**. `changes()` is a `Future`, so
tokio, async-std, smol and a hand-written executor all drive it identically:

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, async)]
#[derive(Debug, Deserialize)]
struct DbConfig { pool_size: u32 }

DbConfig::init_async().await?;
DbConfig::start_watch()?.detach();

let mut changes = DbConfig::changes();

spawn(async move {
    loop {
        let config = changes.changed().await;
        pool.resize(config.pool_size);
    }
});
```

The snapshot current when `changes()` is called counts as already seen, so the
first `changed().await` waits for the *next* reload. Reloads that land while
nothing is awaiting are not queued — waking up to the latest configuration is
what a reader wants, and a queue would hand it stale ones first.

### Where the blocking work goes

Reading configuration touches the filesystem, so `load_async` moves it off the
executor. *Where* is the one genuinely runtime-specific part, so it is
pluggable:

| Setup | `load_async` uses |
|---|---|
| `tokio` feature | `tokio::task::spawn_blocking` |
| [`set_blocking_executor`] installed | that executor |
| neither | a freshly spawned thread |

A configuration load happens at startup and on reload, so a thread per call is a
real answer rather than a placeholder. For async-std or smol, hand the crate its
pool once:

```rust
struct AsyncStd;

impl BlockingExecutor for AsyncStd {
    fn execute(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        async_std::task::spawn_blocking(work);
    }
}

dynamic_config::set_blocking_executor(AsyncStd)?;
```

The watcher itself stays on a plain thread whatever you choose: `notify`'s
channel is synchronous, and keeping it off the runtime means file watching works
whether or not one is running.

[`set_blocking_executor`]: https://docs.rs/dynamic-config/latest/dynamic_config/fn.set_blocking_executor.html

## All of it, or none of it

Two structs over one file reload independently, and for a moment after an edit
one is new while the other is old. Usually nobody notices. When it matters — a
certificate path and the port it is served on — group them:

```rust
let group = ReloadGroup::new()
    .with::<ServerConfig>()
    .with::<TlsConfig>();

group.reload()?;
```

Every member loads and validates before any member is installed, so a failure
anywhere leaves *every* member on its previous snapshot — including the ones
that loaded cleanly. The commits are not one atomic operation; they are three
`Arc` swaps with no fallible work between them, which is the part that actually
goes wrong.

## Command line

Flags sit above the environment and below overrides — a flag is typed by a
person for this one run, and should win over whatever the deployment happens to
export.

```rust
// One call per argument. `None` is a no-op, so unset flags leave the files
// alone and this is safe to run unconditionally.
DbConfig::set_flag("port", matches.get_one::<u16>("port").copied())?;

// Or hand clap the mapping and let it do the plumbing.
DbConfig::bind_clap(&matches, &[("db-host", "host"), ("db-port", "port")])?;

// Or the escape hatch, for keys with no flag of their own.
DbConfig::set_assignments(matches.get_many::<String>("set").into_iter().flatten())?;
```

Keys are relative to the section, so it is `host`, not `db.host`. Values are
read the way environment variables are, so `--set port=8080` and
`APP_DB_PORT=8080` mean the same thing.

`bind_clap` takes **only** arguments that came from the command line. clap's own
`default_value` is indistinguishable from a typed flag in `ArgMatches`, and
letting one outrank a configuration file would invert the whole precedence
order.

The `clap` feature is the only one that pins another crate's major version,
which is why it is separate and opt-in — everything above works without it.

## `.env` files

A `.env` holds *variable names*, not key paths, so it is not another format for
[`files`](#files) — it is the environment layer sourced from disk:

```rust
#[dynamic_config(files = ["config.toml"], key = "db", env = "APP_", env_files = [".env"])]
```

```text
APP_DB_HOST=localhost
APP_DB_POOL__MAX_SIZE=32
```

Same prefix stripping and same nesting as the real environment, merged just
below it — a variable somebody exported for this run beats a file in the
repository. A file that is not there is skipped, like any other.

**It does not touch the process environment.** `dotenvy` and friends call
`setenv`, which changes the environment of the whole program to configure one
struct: a side effect nobody asked for, visible to every library in the process,
and not thread-safe. This reads the file and merges it.

Variable interpolation (`${OTHER}`) and multi-line values are deliberately not
supported. Both are shell features that every `.env` library implements slightly
differently, and a configuration file whose meaning depends on which library
read it is worse than one that refuses.

## Old key paths after a rename

`#[serde(alias)]` covers a renamed *field*. It does not cover a renamed *path*:

```rust
DbConfig::alias("pool.size", "pool.max_size")?;
```

An alias **fills a gap rather than overriding** — a file that has been updated
wins over one that has not, whatever order they merge in, so a deployment
migrating one machine at a time gets no surprise.

The old key stops counting as an unknown key, because an alias that silenced
[typo detection](#what-unknown-key-detection-catches) would make `pool.szie` a
supported spelling. `source_of` reports the file holding the old spelling rather
than the alias, which is the more useful answer: it names the file to edit.

## On a microcontroller

[`dynamic-config-embedded`](dynamic-config-embedded) is a separate `no_std`
crate: no filesystem, no allocator, no runtime.

```rust
static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

SETTINGS.store(Settings { interval_ms: 1000, verbose: false });   // compiled-in defaults
SETTINGS.apply(document, Format::Json)?;                          // from a link, or flash
```

It is not this crate with a feature switched off. A device has no files, no
directories and no environment, and figment is `std` — so what it keeps is the
*shape*: a snapshot in a `static` replaced whole, a bad document leaving the
previous configuration serving, validation, and `changes()` for a task that
would rather await. Storage is a `critical-section` around a plain slot, which
is the one primitive every embedded HAL provides.

CI builds it for `thumbv7em-none-eabihf`, because "it is `no_std`" is a claim a
host build cannot check.

## Bringing your own figment provider

With the `figment` feature, anything figment can read is a source:

```rust
use dynamic_config::figment::providers::{Format as _, Json};

// `.nested()` because this crate reads a top-level key as a section.
let provider = Json::string(document).nested();
let sources = [Source::provider(&provider)];
```

This is the **one** place figment appears in the API, which is why it is behind
a feature: with it off, a figment major bump is not a breaking change here; with
it on, you have opted into that coupling knowingly. figment itself is
re-exported so there is no second version in your graph.

Two things become yours to get right: the provider has to produce the section as
a profile (`.nested()` does that), and `source_of` reports its metadata name, so
a provider that describes itself badly produces a diagnostic that does too.

## Variables that are not yours to name

The [`env`](#env) layer covers the case where the variable names follow from the
prefix, the key and the field. It does not cover the case where they do not:

```text
PORT                 the platform picked it — Heroku, Cloud Run, Fly
DATABASE_URL         a convention older than this program
REDIS_URL            an add-on wrote it into the environment
```

```rust
ServerConfig::bind_env("port", "PORT")?;
DbConfig::bind_env("url", "DATABASE_URL")?;
```

A binding sits just above the prefixed environment layer, because it is the more
specific statement: somebody named that variable on purpose, and the prefixed
one is a convention. It is read at **every** load, so a reload sees a change to
it, and a variable that is not set contributes nothing — which is the point,
since the platform may or may not have set it.

Nested paths work: `bind_env("pool.max_size", "DB_POOL_MAX")`. Binding the same
path twice replaces the first binding rather than layering it — two variables
for one field would have no defensible order between them.

## A schema for the config files

With the `schema` feature, every config type can describe the file it reads, so
an editor completes and validates it:

```rust
#[dynamic_config(files = ["config.json"], key = "db", schema)]
#[derive(Deserialize, JsonSchema)]
struct DbConfig {
    /// Where the database lives.        <- becomes the hover text
    host: String,
    #[config(secret)]                    <- becomes `writeOnly: true`
    password: String,
}

let schema = DbConfig::schema();

// Several types over one file describe that one file together.
let whole = dynamic_config::schema::merge([DbConfig::schema(), ServerConfig::schema()]);
```

What comes out describes the **file**, not the struct — the struct is one
section, and a config file is a map of them, so the schema is the struct's
wrapped under its key.

| Format | How the editor finds it |
|---|---|
| JSON | `"$schema": "./config.schema.json"` as a top-level key |
| YAML | `# yaml-language-server: $schema=./config.schema.json` |
| TOML | `#:schema ./config.schema.json` |

The JSON row is why `$schema` is the one top-level key this crate does not read
as a section: otherwise wiring the schema into the file it describes would stop
the file from loading.

### Nothing is marked required, and that is the point

`schemars` marks every field that is neither `Option` nor `#[serde(default)]` as
required. That is right for a struct and wrong for a config file: the
environment, a flag, an override or a computed default can all supply a value,
and an editor sees none of them. Left in place it would light up every 12-factor
config file in red for values that are perfectly well supplied — so the emitted
schema drops `required` at every depth.

The question a schema cannot answer — *does this actually resolve* — is what
[`check()`](#checking-without-booting) is for, with every layer in view.

## Checking without booting

```text
$ myapp --check
[server]
  host                         set as command-line flag
  port                         from APP_SERVER_*
  tags                         in /etc/myapp/config.json

  hsot: unknown key, did you mean `host`?

  would not load: port: invalid type: found a string, expected u16
```

`check()` reports every key with the layer that supplied it, any key the struct
does not name, and why a load would fail. It **works when the load fails**,
which is the only time it is worth running.

**No values, ever.** A report that showed them would be pasted into an issue
tracker with the database password in it, undoing `#[config(secret)]`.

### What unknown-key detection catches

Top-level keys of the section, compared against the struct's field names —
`db.hsot` is caught, `db.pool.mx_size` is not. A proc-macro sees a field's
*type name*, not its fields, so nothing here knows what lives inside `pool`.

Suggestions use an alignment distance in which a transposition costs one edit,
because `prot` for `port` is how keys actually get mistyped; the threshold
scales with the name, so `id` tolerates one edit and `connection_timeout`
tolerates four.

Detection is skipped entirely when any field is `#[serde(flatten)]`: a flattened
field legitimately absorbs keys the outer struct never names, and reporting
those as typos would be worse than reporting nothing.

## Where did this value come from?

```rust
DbConfig::source_of("port")?;   // Some(Origin::Env("APP_DB_PORT"))
DbConfig::is_set("pool.tls")?;  // false — absent, not "present but false"
```

Both re-read the sources, so they report what the *next* load would see rather
than what the current snapshot holds.

## Reacting to a reload

```rust
DbConfig::on_reload(|previous, current| {
    if previous.pool_size != current.pool_size {
        pool.resize(current.pool_size);
    }
});
```

The callback runs on whichever thread performed the reload — the watcher
thread, usually — so keep it short. Installing the first snapshot is not a
reload, so `init()` does not fire it. With the `async` feature, `changes()` is
the same idea for a task that would rather await than be called back.

## Without the macro

The engine is public and usable on its own:

```rust
use dynamic_config::{load, ConfigCell, Format, LoadSpec, Source};
use serde::Deserialize;

#[derive(Deserialize)]
struct Db { host: String }

static DB: ConfigCell<Db> = ConfigCell::new();

let sources = [Source::inline(r#"{"db": {"host": "localhost"}}"#, Format::Json)];
let db: Db = load(&LoadSpec { key: "db", sources: &sources, env_prefix: None })?;

DB.store(db);
assert_eq!(DB.load().unwrap().host, "localhost");
# Ok::<(), dynamic_config::Error>(())
```

## Errors

One error type; `figment::Error` never reaches a signature, so a figment major
version bump is not automatically a breaking change here. Every error carries
the key path and the source that set the value:

```text
pool.max_size: invalid type: found a string, expected u16 (from APP_DB_)
```

**The offending value is not in the message.** The key, what kind of thing was
there, and the type that was wanted are all there — everything needed to fix it.
The value is not, because a password pasted into a numeric field would otherwise
land in a log line, and every other diagnostic here goes to some length to make
sure that cannot happen.

`Error::kind()` returns `Io`, `Parse`, `Missing`, `Type`, `Env`, `Invalid`,
`Remote`, `Decrypt` or `Backend`.

## Cargo features

| Feature | Default | Effect |
|---|---|---|
| `json` | ✅ | `.json` sources |
| `toml` | | `.toml` sources |
| `yaml` | | `.yaml` / `.yml` sources |
| `watch` | | `start_watch()` and the file watcher |
| `async` | | `load_async`, `init_async`, `changes`, `AsyncRemoteSource` — no runtime dependency |
| `tokio` | | `async`, plus tokio's blocking pool instead of a thread per load |
| `clap` | | `bind_clap` |
| `schema` | | `schema()` — a JSON Schema for the config files |
| `decrypt` | | `Decryptor` and `set_decryptor`, for a scheme of your own |
| `age` | | `decrypt`, plus transparent decryption and encryption of `age` files |
| `dotenv` | | `env_files` — a `.env` read as the environment layer |
| `figment` | | `Source::provider`, and figment re-exported |
| `tracing` | | Watcher diagnostics via `tracing` instead of stderr |
| `full` | | all of the above |

Using a format, `watch` or `tokio` whose feature is off is a compile error
naming the feature to add — not a runtime surprise on the one machine that
reads YAML.

## Minimum supported Rust version

| Configuration | MSRV |
|---|---|
| any format, `tokio`, `tracing`, `dotenv`, `figment` | 1.71 |
| `watch` enabled | 1.85 (`notify 8` requires it) |
| `schema` enabled | 1.74 (`schemars` requires it) |
| `age` enabled | 1.85 — measured, not declared (see below) |
| the companion crates (etcd, Consul, Vault, Firestore) | 1.85 |
| `dynamic-config-nats`, `-redis`, `-s3` | 1.88 (their clients require it) |
| `dynamic-config-embedded` | 1.83 (`core::error::Error` in `no_std` needs 1.81) |

Only `watch`, `schema` and `age` raise the floor for the core crate; `tokio`
does not.

`age` declares 1.74 for itself, and the figure above is 1.85 because that is
what actually builds: `age` pulls `rust-embed` for its translations, which pulls
`sha2 0.11`, which is edition 2024. The number here is the one the CI job
verifies against a real toolchain, not the one a manifest claims. A companion
pays for what it pulls in — a gRPC stack, a streaming client, an HTTP client —
and the core stays where it is.

MSRV is treated as a breaking change, and both figures are verified in CI
against the real toolchains.

Contributors: this repository sets
`resolver.incompatible-rust-versions = "fallback"` in `.cargo/config.toml`.
Without it, cargo resolves to the newest release of every transitive dependency
and the floor silently becomes 1.85 — `hashbrown 0.17`, reached through `toml`,
requires edition 2024. Generating the lockfile needs cargo 1.84 or newer.

## Limitations

- **Every top-level key in a config file must be a table**, with one exception:
  `$schema`, so a JSON file can point at the schema that describes it. Sections
  are figment profiles and a profile has to be a map, so a stray
  `"_comment": "..."` at the top level is an error — one that now names the key
  and says why.
- TOML datetimes are not modelled and deserialize as a table.
- The macro refers to the crate as `::dynamic_config`, so renaming the
  dependency is not supported.
- Error messages name the environment *prefix* rather than the exact variable,
  because that is the granularity figment reports.

## Examples

Twenty-six of them, each showing one idea. All run from the workspace root.

### Getting started

| Example | Features | Shows |
|---|---|---|
| [`basic`](dynamic-config/examples/basic.rs) | `json` | Load once, read the snapshot. |
| [`sections`](dynamic-config/examples/sections.rs) | `json`, `watch` | Several config types over one set of files, each owning its own key, files and watcher. |
| [`errors`](dynamic-config/examples/errors.rs) | `json` | Every `ErrorKind`, what each one calls for, and reading `path` and `origin`. |

### Where values come from

| Example | Features | Shows |
|---|---|---|
| [`layers`](dynamic-config/examples/layers.rs) | `json` | One key climbing all five layers, with `source_of` naming each. |
| [`env_only`](dynamic-config/examples/env_only.rs) | `json` | No files at all: the 12-factor arrangement, nested and list values included. |
| [`discovery`](dynamic-config/examples/discovery.rs) | `json` | `name` + `paths` across two directories, plus a profile overlay. |
| [`cli`](dynamic-config/examples/cli.rs) | `clap`, `json` | Flags over the environment, `--set key=value`, and `--check` instead of booting. |

### Reloading

| Example | Features | Shows |
|---|---|---|
| [`hot_reload`](dynamic-config/examples/hot_reload.rs) | `watch`, `toml` | Edit a file and watch the snapshot follow. |
| [`async_reload`](dynamic-config/examples/async_reload.rs) | `async`, `watch`, `json` | A task awaiting reloads instead of polling for them. |
| [`group`](dynamic-config/examples/group.rs) | `json` | Two config types reloading as one step, or not at all. |

### Getting it right

| Example | Features | Shows |
|---|---|---|
| [`validation`](dynamic-config/examples/validation.rs) | `json` | Rejecting a configuration where every field is valid and the whole is not. |
| [`secrets`](dynamic-config/examples/secrets.rs) | `json` | `#[config(secret)]`, and precisely what it does and does not cover. |
| [`testing`](dynamic-config/examples/testing.rs) | `json` | Pinning configuration under test with the override layer. |

### With a web framework

| Example | Features | Shows |
|---|---|---|
| [`axum_hello`](dynamic-config/examples/axum_hello.rs) | `watch`, `json` | A handler that reads `current()` per request, a `/config/check` probe, and why the listen port is start-up configuration. |
| [`actix_hello`](dynamic-config/examples/actix_hello.rs) | `watch`, `json` | The same across Actix's worker threads, and why configuration does not belong in `web::Data`. |

### On a runtime

| Example | Features | Shows |
|---|---|---|
| [`tokio_runtime`](dynamic-config/examples/tokio_runtime.rs) | `tokio`, `watch`, `json` | Two readers each waking on their own `changes()` handle, with tokio's blocking pool wired in for free. |
| [`smol_runtime`](dynamic-config/examples/smol_runtime.rs) | `async`, `watch`, `json` | The whole async surface on smol, with smol's `unblock` installed as the blocking executor and no tokio in the build. |
| [`embassy_runtime`](dynamic-config/examples/embassy_runtime.rs) | `async`, `json` | Embassy — an executor for microcontrollers, with no threads and no reactor — driving `changes()`, and why two rapid reloads are one wakeup. |

### Reaching further

| Example | Features | Shows |
|---|---|---|
| [`units`](dynamic-config/examples/units.rs) | `json` | `"30s"` and `"64MiB"`, from files and from the environment. |
| [`generic`](dynamic-config/examples/generic.rs) | `json` | `Db<Postgres>` and `Db<Mysql>` with separate snapshots. |
| [`persistence`](dynamic-config/examples/persistence.rs) | `json` | Writing back atomically, and reading keys with no field. |
| [`remote`](dynamic-config/examples/remote.rs) | `json` | A `RemoteSource` of your own: explicit fetch, where it sits between the layers, an unreachable store, and a watch loop pushing through `apply_remote`. |
| [`last_known_good`](dynamic-config/examples/last_known_good.rs) | `json` | All three `cache_mode`s against the same broken file, with each cache file printed. |
| [`encrypted`](dynamic-config/examples/encrypted.rs) | `age`, `json` | A `secrets.json.age` next to a plain `config.json`: generated key, real ciphertext, and what the wrong key looks like. |
| [`schema`](dynamic-config/examples/schema.rs) | `schema`, `json` | A JSON Schema for the file two config types share, with secrets marked and `required` dropped. |
| [`no_macro`](dynamic-config/examples/no_macro.rs) | `json` | `load`, `LoadSpec`, `Layer` and `ConfigCell` without the attribute. |

```sh
cargo run -p dynamic-config --example errors      --features json
cargo run -p dynamic-config --example layers      --features json
cargo run -p dynamic-config --example cli         --features clap,json -- --check
cargo run -p dynamic-config --example hot_reload  --features watch,toml
cargo run -p dynamic-config --example remote      --features json
cargo run -p dynamic-config --example last_known_good --features json
cargo run -p dynamic-config --example schema      --features schema,json
cargo run -p dynamic-config --example encrypted   --features age,json
cargo run -p dynamic-config --example axum_hello  --features watch,json
cargo run -p dynamic-config --example actix_hello --features watch,json
cargo run -p dynamic-config --example tokio_runtime --features tokio,watch,json
cargo run -p dynamic-config --example smol_runtime --features async,watch,json
cargo run -p dynamic-config --example embassy_runtime --features async,json
APP_ENV=production cargo run -p dynamic-config --example discovery --features json
```

## Not planned

Each of these is a real request with a real answer. They are refused rather than
unbuilt, so that nobody spends an afternoon discovering the reason — and each
says what would reopen it.

### Nested profiles from figment

figment's profiles are a general mechanism. This crate spends them on
**sections** — `key = "db"` selects the `db` profile — and re-implements the
profile *idea* on top with [`profile_env`](#profile_env) and sibling files
(`config.production.toml`). So a provider handed to
[`Source::provider`](#bringing-your-own-figment-provider) cannot carry its own
profiles through.

The difficulty is not any one part; it is that `select(key)`, the section
mapping, `profile_env`, sibling files, `check()`, `source_of` and every
diagnostic that names a section all assume the current arrangement. Changing it
means giving sections a different mechanism and rewriting the layering
underneath everything that reads well today.

**What would reopen it:** a figment provider whose own profiles you need, where
`Source::provider` plus `profile_env` genuinely cannot express what you are
after.

### Case-insensitive keys

Viper lowercases everything. It hides typos — `Prot` and `port` become the same
key, so [unknown-key detection](#what-unknown-key-detection-catches) can never
tell you about the first — and it cannot round-trip: a configuration read and
written back comes out in different case from the one a person wrote.

**What would reopen it:** nothing. This one is a principle rather than a cost.

### HCL, Java properties, INI

Each is a parser and a set of edge cases for a format nobody here has asked for,
and none of them is something figment provides.

**The answer that is not a fork:**
[`Source::provider`](#bringing-your-own-figment-provider) takes any figment
provider, so a crate that parses one of these wires in without this one growing
a dependency.

### Independent instances

Viper needs them because its default instance is a global. Here every
configuration type already has its own storage, keyed by the type — the same
isolation without the bookkeeping.

### Inferring a type from a default value

serde already knows the type. Viper's `SetTypeByDefaultValue` exists because
Go's `map[string]interface{}` does not.

### A service-account JSON key for Firestore

Signing one means an RS256 stack inside a configuration library, and Google's
own guidance is that a downloaded key is the option of last resort.
[Workload identity](dynamic-config-firestore/README.md#authenticating) covers GKE, Cloud
Run, GCE and Cloud Functions; anything else can mint a token outside the process
and pass it in.

## Roadmap

[ROADMAP.md](ROADMAP.md) is what might still be built, and why each item is not
obvious. It is short on purpose.

## Contributing and security

[docs/CONTRIBUTOR-ONBOARDING.md](docs/CONTRIBUTOR-ONBOARDING.md) is a tour of
every crate and module — what each does and where you would change it.
[CONTRIBUTING.md](CONTRIBUTING.md) has what a change should carry and what is
load-bearing enough to argue about. [SECURITY.md](SECURITY.md) states the
properties this crate tries to keep — and the ones it explicitly does not —
along with how to report a vulnerability privately.

`just check` runs what CI runs; `just containers` adds the suites that need a
Docker daemon.

## License

MIT

[`Watching`]: https://docs.rs/dynamic-config/latest/dynamic_config/struct.Watching.html
[`config`]: https://docs.rs/config
[figment]: https://docs.rs/figment
[`figment`]: https://docs.rs/figment
[Viper]: https://github.com/spf13/viper

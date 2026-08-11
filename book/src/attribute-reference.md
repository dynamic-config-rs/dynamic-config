# Attribute reference

`#[dynamic_config]` takes **no arguments**. The attribute declares — *this
type is a configuration* — and generates the part a runtime value cannot
provide: the type's snapshot storage, the accessors over it, its runtime
layers, and the diagnostics that follow from the fields. Where the
configuration *comes from* is runtime data, so it lives in runtime code: on
the `Builder` the generated `builder(key)` returns.

```rust
use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
}

DatabaseConfig::builder("db")
    .file("config.toml")
    .file("secrets.json")
    .env("APP_")
    .init()?;

let config = DatabaseConfig::current();
```

Anything between the attribute's parentheses is a compile error whose
message maps each old argument to its builder method — the same map as
[the migration table below](#migration-from-attribute-arguments).

A successful `init()` does one more thing than install: it **remembers the
builder** as the type's configuration. That is what lets the generated
`source_of`, `check`, `explain`, `prepare` and the remote-reload methods
answer for the configuration the process actually runs on, without being
handed the builder again. Keep the builder around anyway if you intend to
`watch` with it.

The struct must implement `serde::Deserialize` and be
`Send + Sync + 'static`. Type and const parameters work — see
[Generic Configs](generic-configs.md); a lifetime parameter is rejected at
compile time, because the snapshot outlives every borrow that could name
one.

## The field attribute

| Field attribute | Form | Effect |
|---|---|---|
| [`secret`](validation-diagnostics.md#configsecret) | `#[config(secret)]` | The generated `Debug` prints `***` for the field; `#[derive(Debug)]` alongside it is a compile error. The field stays out of a redacted cache, comes back already `***` from `explain`, and is marked `writeOnly` in the schema. |

## What the attribute generates

Every method here is generated on the annotated type. The ones marked with
a feature exist only when that Cargo feature is enabled — there is no
argument to opt in with; an unused method costs nothing.

| Method | Feature | Description |
|---|---|---|
| `builder(key) -> Builder<Self>` | | Where everything starts: a [`Builder`](#the-builder) wired to this type's storage — its `init()` installs into the snapshot `current()` reads. |
| `current() -> Arc<Self>` | | The current snapshot, one atomic load. Panics before anything installed one. |
| `try_current() -> Option<Arc<Self>>` | | The current snapshot, or `None`. |
| `replace(config)` | | Atomically swap in a new snapshot. Readers holding an `Arc` keep their own generation. |
| `prepare() -> Result<Commit, Error>` | | Load and validate through the remembered builder without installing — the fallible half of a reload, for a `ReloadGroup`. |
| `on_reload(hook)` | | Run a callback on every later reload, for the life of the process. |
| `on_reload_scoped(hook) -> HookGuard` | | The same, until the guard is dropped. |
| `set_default(path, value)` / `set_override(path, value)` | | The two runtime layers bracketing everything else: a fallback used only when nothing supplies the key, and a value that wins over every file and variable. |
| `set_defaults(value)` | | Every field of a `Serialize` value as defaults, at once. |
| `set_flag(path, value)` / `set_assignments(items)` | | The command-line layer: one flag, or `--set key=value` strings. |
| `clear_defaults()` / `clear_overrides()` / `clear_flags()` | | Drop a runtime layer again. |
| `bind_env(path, variable)` / `clear_env_bindings()` | | Bind a field to a variable that is not yours to name — `PORT`, `DATABASE_URL`. |
| `alias(from, to)` / `clear_aliases()` | | Keep an old key path working after a rename; fills a gap rather than overriding. |
| `source_of(path)` / `is_set(path)` | | Which layer supplies a key, and whether anything does — through the remembered builder. |
| `snapshot() -> Result<Snapshot, Error>` | | Resolve without deserializing, for keys with no field. |
| `check() -> Result<Report, Error>` | | What the configuration resolves to, and whether it would load — works when the load fails, which is when it is worth running. |
| `explain(path) -> Result<Explanation, Error>` | | Every layer's answer for one path, values included — secret fields come back already `***`. |
| `set_remote(source)` / `refresh_remote()` / `apply_remote(document)` / `clear_remote()` | | Install a remote store, fetch from it explicitly, push a watched document through the reload path, drop what it gave. |
| `bind_clap(matches, bindings)` | `clap` | Copy named clap arguments into the flags layer — only ones that really came from the command line. |
| `load_async()` / `init_async()` | `async` | The remembered builder's load and init, off the async executor. |
| `changes()` | `async` | A handle woken by every later reload; a `Future`, so any executor drives it. |
| `set_remote_async(source)` / `refresh_remote_async()` | `async` | The same remote surface, for a store whose client is async. |

Three names from before the split are **not** on this list, deliberately:
`load` and `init` live on the builder, because a load needs to know its
sources; `start_watch` is now the builder's `watch(debounce)`, because
watching re-runs a load.

## The Builder

`builder(key)` returns a `Builder<Self>` — the whole source side of the
configuration, chosen at runtime. Methods take and return `self`, are
infallible, and defer every check to load time: a missing file or an
unsupported extension is a load-time answer, the same as everywhere else.
The bare `Builder::new(key)` is the same type with no config type attached:
its `load()` works, and everything that needs somewhere to install refuses
with an error saying to start from the generated `builder()`.

### Sources and options

| Method | Description |
|---|---|
| `file(path)` | Adds a configuration file. Merged in call order; later files win. The format comes from the extension, at load time; a missing file is skipped. |
| `encrypted_file(path)` | Adds an encrypted file — `secrets.json.age`; the format comes from the extension under the suffix. Needs the `decrypt` feature; see [Encryption](encryption.md). |
| `discover(name, paths)` | Looks for `{name}.{ext}` in each directory, below any explicitly listed files. See [Profiles & Discovery](profiles-and-discovery.md#name--paths). |
| `env(prefix)` | The environment layer: `prefix` plus the key, so `env("APP_")` with key `db` reads `APP_DB_*`. |
| `nest(separator)` | The nesting separator inside variable names; `"__"` unless said. |
| `allow_empty_env()` | Treats `FOO=` as set-to-empty rather than unset. |
| `strict_env()` | Refuses ambiguous environment spellings — `off`, `no`, `nil` — with an error naming the variable. |
| `env_file(path)` | A `.env` file read as the environment layer, just below the real thing. Needs the `dotenv` feature at load time. |
| `profile_env(variable)` | The environment variable naming the active profile, as in `profile_env("APP_ENV")`. |
| `cache(path, mode)` | A last-known-good cache: written after every clean `init` or watch reload, recovered from when the sources will not load. `mode` is a [`CacheMode`](persistence.md#cache-modes). |
| `validate(f)` | Application-level validation, `fn(&T) -> Result<(), Error>`, run after deserializing and before anything installs. A reload it refuses keeps the previous snapshot. |

### Loading and installing

| Method | Feature | Description |
|---|---|---|
| `load() -> Result<T, Error>` | | Reads the sources and deserializes, installing nothing. |
| `init() -> Result<(), Error>` | | Loads, installs as the type's snapshot, remembers the builder — and recovers from the cache when the sources are broken. |
| `reload() -> Result<(), Error>` | | One reload by hand: load, validate, install, rewrite the cache. A failure installs nothing. |
| `prepare() -> Result<Commit, Error>` | | Load and validate now, install later — what a `ReloadGroup` drives. |
| `watch(debounce) -> io::Result<WatchHandle>` | `watch` | Reloads on file changes until the returned handle is dropped. One watcher per type, whichever surface starts it. |
| `watch_with(debounce, mode)` | `watch` | The same, with the detection strategy chosen explicitly — `WatchMode::Poll` is what network and overlay filesystems need. |
| `load_async()` / `init_async()` | `async` | The same, off the async executor. |

### Diagnostics on the builder

The same questions the generated methods answer, against this builder's
sources — useful before anything is installed, or on a bare
`Builder::new`.

| Method | Feature | Description |
|---|---|---|
| `source_of(path)` / `is_set(path)` | | Which layer supplies a key, and whether anything does. |
| `snapshot()` | | The resolved section, without deserializing. |
| `check()` | | The full report: every key's provenance, unknown keys, whether a load would succeed. |
| `explain(path)` | | Every layer's answer for one path. A generated `builder()` knows the secret fields and hands their paths back already `***`; a bare `Builder::new` does not — redact accordingly. |
| `schema()` | `schema` | A JSON Schema for the file this section lives in, with `#[config(secret)]` fields marked `writeOnly`. Needs `T: JsonSchema`. |

### What only the generated builder can do

The generated `builder()` carries what the macro knows and a bare
`Builder::new` cannot: where to install, the field names for unknown-key
detection, and which fields are `#[config(secret)]`. The consequences are
deliberate rather than incidental:

- `init`, `reload`, `prepare` and `watch` on a bare builder refuse — there
  is nowhere to install. `load()` is the bare builder's whole job.
- A redaction-dependent cache mode (`CacheMode::Redacted`,
  `CacheMode::Fingerprint`) on a bare builder is refused at `init` rather
  than silently caching everything: knowing there are *no* secret fields is
  knowledge, and only the generated builder has it. `CacheMode::Full`,
  spelled out, still works.
- A bare builder's `check()` reports no unknown keys, and its `schema()`
  marks nothing `writeOnly`.

## Migration from attribute arguments

Every source argument the attribute used to take moved to the builder. The
semantics are unchanged — precedence, merging, profiles, cache behaviour
and watch behaviour all mean what they meant; only where you *state* them
moved.

| Was | Is now |
|---|---|
| `key = "db"` | `builder("db")` — the argument. |
| `files = ["a.toml", "b.json"]` | `.file("a.toml").file("b.json")` — call order is merge order. |
| `files = []` | A builder with no `.file(..)` calls. |
| `name = "config"`, `paths = [..]` | `.discover("config", ["/etc/app", "."])` |
| `env = "APP_"` | `.env("APP_")` |
| `nest = "___"` | `.nest("___")` |
| `allow_empty_env` | `.allow_empty_env()` |
| `strict_env` | `.strict_env()` |
| `env_files = [".env"]` | `.env_file(".env")`, once per file. |
| `profile_env = "APP_ENV"` | `.profile_env("APP_ENV")` |
| `cache = "path"`, `cache_mode = "full"` | `.cache("path", CacheMode::Full)` — the mode is an enum now and always stated; `Redacted` is still the usual choice. |
| `validate` | `.validate(f)` — a function you pass, not a method the macro resolves. `Error::ok_or_invalid(value.validate())` adapts a validator library's `Result`. |
| `watch`, `debounce = 250` | `builder.watch(Duration::from_millis(250))?` on the builder `init()` ran on. The handle still stops the watcher on drop, and still has `detach()`. |
| `poll`, `poll_interval = 2000` | `builder.watch_with(debounce, WatchMode::Poll { interval: Duration::from_secs(2) })?` |
| `diff` | `dynamic_config::changed_paths(old, new)` in an `on_reload` hook — see [Key-level diffs](validation-diagnostics.md#key-level-diffs). |
| `save` | The free functions `dynamic_config::save(&value, path, format, key)`, `save_new`, `save_encrypted` — see [Persistence & Writing](persistence.md#save). |
| `schema` | `builder.schema()` — see [Schema](schema.md#the-schema-method). |
| `async` | Nothing: with the `async` feature enabled, the async methods are always generated. |
| `Config::init()` / `Config::load()` (generated) | `builder.init()` / `builder.load()` |
| `Config::start_watch()` (generated) | `builder.watch(debounce)` |

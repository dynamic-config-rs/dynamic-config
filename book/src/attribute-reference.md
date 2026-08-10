# Attribute reference

Every argument `#[dynamic_config(..)]` accepts, and every field attribute.

## At a glance

| Argument | Form | Requires | Default |
|---|---|---|---|
| [`files`](sources-and-precedence.md#files) | `files = ["a.toml", "b.json"]` | one of `files` / `name`+`paths` | — |
| | `files = []` | | no files at all, on purpose |
| [`name`](profiles-and-discovery.md#name--paths) | `name = "config"` | `paths` | — |
| [`paths`](profiles-and-discovery.md#name--paths) | `paths = ["/etc/app", "."]` | `name` | — |
| [`key`](sources-and-precedence.md#key) | `key = "db"` | always required | — |
| [`env`](sources-and-precedence.md#env) | `env = "APP_"` | | no environment layer |
| [`nest`](sources-and-precedence.md#nest) | `nest = "__"` | `env` | `"__"` |
| [`allow_empty_env`](sources-and-precedence.md#allow_empty_env) | flag | `env` | off — `FOO=` is unset |
| [`profile_env`](profiles-and-discovery.md#profile_env) | `profile_env = "APP_ENV"` | | no profile overlay |
| [`watch`](hot-reload.md#watch) | flag | `watch` feature | off |
| [`debounce`](hot-reload.md#debounce) | `debounce = 250` | `watch` | `250` ms |
| [`poll`](hot-reload.md#poll--poll_interval) | flag | `watch` | native backend |
| [`poll_interval`](hot-reload.md#poll--poll_interval) | `poll_interval = 2000` | `watch` | `2000` ms with `poll` |
| [`diff`](validation-diagnostics.md#diff) | flag | | off |
| [`validate`](validation-diagnostics.md#validate) | flag | a `validate()` on the type | off |
| [`save`](persistence.md#save) | flag | `Self: Serialize` | off |
| [`cache`](persistence.md#cache) | `cache = "/var/lib/app/last.json"` | | no cache — a bad start fails |
| [`cache_mode`](persistence.md#cache_mode) | `cache_mode = "redacted"` | `cache` | `"full"` |
| [`env_files`](sources-and-precedence.md#env_files) | `env_files = [".env"]` | `dotenv` feature + `env` | none |
| [`schema`](schema.md#the-schema-attribute) | flag | `schema` feature + `Self: JsonSchema` | off |
| [`async`](async-runtimes.md#the-async-attribute) | flag | `async` feature | off |

| Field attribute | Form | Effect |
|---|---|---|
| [`secret`](validation-diagnostics.md#configsecret) | `#[config(secret)]` | `Debug` prints `***`; forbids `#[derive(Debug)]` |

Anything else is a compile error listing the arguments that exist.

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
| `on_reload_scoped(f) -> HookGuard` | ✅ | The same, until the guard is dropped. |
| `prepare() -> Result<Commit, Error>` | ✅ | Load without installing, for a `ReloadGroup`. |
| `set_default` / `set_override` / `set_flag` | ✅ | The three runtime layers. |
| `set_defaults(value)` | ✅ | Every field of a `Serialize` value as defaults, at once. |
| `set_assignments(["k=v"])` | ✅ | Apply `--set key=value` strings. |
| `bind_env(path, "PORT")` / `clear_env_bindings()` | ✅ | Bind a field to an environment variable by name. |
| `clear_defaults` / `clear_overrides` / `clear_flags` | ✅ | Drop them again. |
| `start_watch() -> io::Result<WatchHandle>` | `watch` | Reload on file changes. A second call while one runs is `AlreadyExists`. |
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

# API Reference

Everything the package exports, in one place. Where a call has an async
twin it sits in the same row, because the pair is the point: the
synchronous one is right from a thread, a script or a test, and the
`_async` one hands the blocking half to an executor so the event loop is
never the thing waiting.

```python
from dynamic_config import DynamicConfig, dynamic_config, set_executor, changed_paths
```

## `DynamicConfig(model, key, *, executor=None)`

`Generic[M]`, so every method that hands a model back hands back *your*
model rather than `Any`.

| | |
|---|---|
| `model` | the schema class: a `dataclasses.dataclass`, a Pydantic model, or a Pydantic dataclass — see [What a schema may be](types.md#what-a-schema-may-be) |
| `key` | the section this configuration reads (`[db]` in a TOML file) |
| `executor` | which pool runs the blocking half of the async calls; `None` follows [`set_executor`](#set_executorexecutor) |

### `DynamicConfig.from_settings(model, key, *, executor=None)`

A configuration whose sources come from a `pydantic_settings.BaseSettings`
class's own `SettingsConfigDict` — its files, its `.env`, its variable
names — so an existing settings class keeps working and gains layering,
provenance and hot reload. Refuses what has no engine equivalent
(`secrets_dir`, `cli_parse_args`, an overridden
`settings_customise_sources`) instead of dropping it. Chain more sources
onto the result as usual. See
[pydantic-settings](types.md#pydantic-settings).

### Sources

Each returns the configuration, so they chain. All of them raise once
anything has loaded — sources are how a configuration is *identified*.

| Method | Effect |
|---|---|
| `file(path)` | Adds a file. Merged in call order, later wins; a missing one is skipped |
| `discover(name, paths)` | Looks for `{name}.{ext}` in each directory, *below* listed files |
| `env(prefix)` | The environment layer: `prefix` plus the section key (`APP_DB_*`) |
| `nest(separator)` | The separator that means nesting inside a variable name; `__` unless said |
| `allow_empty_env()` | Treats `FOO=` as set-to-empty rather than unset |
| `strict_env()` | Refuses ambiguous spellings — `off`, `no`, `nil` — naming the variable |
| `env_file(path)` | A `.env` read as the environment layer, just below the real one |
| `profile_env(variable)` | The variable naming the active profile, for sibling files |
| `cache(path, mode="redacted")` | A last-known-good cache; `redacted`, `full` or `fingerprint` |

### Lifecycle

| Synchronous | Async twin | Does |
|---|---|---|
| `init()` | `init_async()` | Loads, validates, installs |
| `init_and_current()` | `init_and_current_async()` | Both of the above, for the code that wants the values rather than the object |
| `load()` | `load_async()` | Loads and validates, installs **nothing**; returns the candidate |
| `reload()` | `reload_async()` | Loads, validates, installs again, rewrites the cache |
| `current()` | — | The installed model. One attribute lookup; raises `NotInitialisedError` before the first load |
| `try_current()` | — | The same, or `None` |
| `replace(model)` | — | Installs a model you built, firing the hooks |
| `changed(timeout=None)` | `changed_async(timeout=None)` | Blocks until the next install; `None` on timeout |
| — | `changes()` | An async iterator over every install from here on |
| `watch(debounce=0.25, poll_interval=None)` | `watch_async(…)` | Starts a watcher; returns a [`Watch`](#watch) |
| `on_reload(hook)` | — | Runs `hook(old, new)` after every install; returns a [`HookGuard`](#hookguard). Usable as a decorator |
| `on_change(*paths)` | — | The decorator form of the same, firing only when one of `paths` moved. See [Callbacks](callbacks.md) |

`current()` and `try_current()` have no async twin because there is
nothing to await: the model is cached on the object, so the read is an
attribute lookup on the loop and on a thread alike.

`watch` has a twin for a narrower reason than the others: the watcher is
a thread either way, so what `watch_async` moves off the loop is only
*starting* it — resolving directories, registering each with the
notification backend, spawning the carrier thread. That is syscalls
rather than I/O, and it measures a fraction of a millisecond natively;
but it grows with the number of directories, and `poll_interval` takes a
baseline scan of everything it watches first, which is single-digit
milliseconds over a large directory and worse over the network
filesystems that are the reason to poll. A startup handler runs once and
would survive either call; the async one is the same work with the wait
on a worker.

`Watch.stop()` has no twin, and that is not an omission: it drops the
backend, which closes the channel the watcher thread is parked on, and
returns without joining it or waiting out a debounce window. Under a
tenth of a millisecond, so a shutdown handler can call it directly.

### Runtime layers

The two layers that bracket every source: defaults lose to everything,
overrides beat everything.

| Method | Effect |
|---|---|
| `set_default(path, value)` | A fallback the program computes and a file need not state |
| `set_defaults(mapping_or_model)` | Every field of a mapping or model, at once |
| `set_override(path, value)` | Outranks every source — what makes a test authoritative |
| `set_assignments(["key=value", …])` | `--set`-style strings |
| `clear_defaults()` / `clear_overrides()` / `clear_assignments()` | Empty one layer |
| `alias(old, new)` | Keeps files written before a rename working |
| `bind_env(path, variable)` | Maps one field to one variable by name — `PORT`, `DATABASE_URL` |

These take effect on the next load, so a `set_override` after `init()`
wants a `reload()` behind it.

### Diagnostics

| Method | Returns |
|---|---|
| `source_of(path)` | [`Origin`](#origin) — which layer would supply it — or `None` |
| `is_set(path)` | Whether anything supplies it |
| `explain(path)` | [`Explanation`](#explanation) — every layer's answer, secrets redacted |
| `check()` | [`Report`](#report) — would it load, and is anything unknown |
| `snapshot()` | [`Snapshot`](#snapshot) — the resolved section as data |

### Properties

| | |
|---|---|
| `key` | The section key |
| `model` | The Pydantic class |
| `generation` | How many models have been installed; zero before the first |

## Module functions

### `set_executor(executor)`

Process-wide choice of which thread pool pays for the blocking half of
the async calls. `None` restores the loop's own. Waits deliberately stay
on the loop's default executor — see
[Async & asyncio](async.md#which-pool-pays-for-the-blocking-half).

### `secret_paths(model)`

Every dotted path in `model` that holds a `SecretStr` or `SecretBytes` —
through `Optional`, unions, containers, nested models, Pydantic
dataclasses and `RootModel`. This is what seeds the redaction, and it is
derived rather than declared, so nobody keeps a second list in step with
the first. A field lists **every** name a file could carry it under (each
alias and the field name), because a secret spelled the other way is
still a secret; see [Aliases](types.md#aliases-in-all-four-shapes).

### `changed_paths(previous, current)`

Which paths differ between two models (or mappings), as
[`Change`](#change) values. Paths only, never values — including for
secrets, whose values are compared but never reported.

### `@dynamic_config(...)`

Attaches a configuration to a model class and returns the class.

| Argument | Default | Meaning |
|---|---|---|
| `key` | required | The section key |
| `files` | `()` | Files, in merge order |
| `discover` | `None` | `(name, paths)` |
| `env` | `None` | Environment prefix |
| `nest` | `None` | Nesting separator |
| `allow_empty_env` | `False` | |
| `strict_env` | `False` | |
| `env_files` | `()` | `.env` files |
| `profile_env` | `None` | Variable naming the profile |
| `cache` / `cache_mode` | `None` / `"redacted"` | Last-known-good cache |
| `init` | `False` | Load at decoration — off, because import time is not load time |
| `watch` | `None` | Start a detached watcher with this debounce |

It attaches `config`, `current`, `try_current`, `reload`, `source_of` and
`explain` to the class, and refuses a model that declares a field with
one of those names.

## Types

### `Origin`

`kind` (`file`, `env`, `inline`, `remote`, `runtime`, `unknown`),
`detail` (the path, the variable, the store). `str()` renders it as the
crate does: *in /etc/app.toml*, *from APP_DB_PORT*.

### `Explanation`

`path`, `rows` (a tuple of `Contribution`: `layer`, `value`, `origin`),
`winner`. `str()` is the table; `repr()` is shape only, because a repr
lands in a log by accident and this is the one object that carries
values.

### `Report`

`key`, `resolved` (tuple of `Resolved`: `path`, `origin`), `unknown`
(tuple of `UnknownKey`: `path`, `suggestion`), `failure`, and the
`is_clean` property.

### `Snapshot`

`to_dict()`, `source_of(path)`, `contains(path)`, `leaf_paths()`,
`top_level_keys()`, `is_empty()`, `diff(other)` → `Change` values.

### `Change`

`path` and `kind` (`added`, `removed`, `changed`).

### `Watch`

`running`, `stop()`, `detach()`, and a context manager that stops on
exit.

### `HookGuard`

`close()`, `hook`, and a context manager that unregisters on exit. It is
also callable, forwarding to the hook — which is what lets
`@config.on_reload` decorate a function without taking it away.

## Exceptions

`DynamicConfigError` is the base — catching it catches everything. Each
instance carries `kind`, `path`, `origin_kind` and `origin`.

| Class | Raised when |
|---|---|
| `IoError` | A source exists but could not be read |
| `ParseError` | A source is not valid in its format |
| `MissingError` | A required value is supplied by nothing |
| `TypeMismatchError` | A value cannot become the requested type |
| `EnvError` | An environment variable could not be interpreted |
| `InvalidError` | The configuration as a whole was rejected — Pydantic's report is on `.errors`, scrubbed of input values |
| `RemoteError` | A remote store could not be read |
| `DecryptError` | An encrypted source could not be decrypted |
| `BackendError` | The engine refused — a source added after loading, for instance |
| `NotInitialisedError` | `current()` before the first successful load |

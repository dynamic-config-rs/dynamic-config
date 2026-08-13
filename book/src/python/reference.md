# API Reference

Everything the package exports, in one place. Where a call has an async
twin it sits in the same row, because the pair is the point: the
synchronous one is right from a thread, a script or a test, and the
`_async` one hands the blocking half to an executor so the event loop is
never the thing waiting.

```python
from dynamic_config import DynamicConfig, dynamic_config, set_executor, changed_paths
```

## `DynamicConfig(model, key, *, executor=None, secrets=())`

`Generic[M]`, so every method that hands a model back hands back *your*
model rather than `Any`.

| Parameter | Default | Meaning |
|---|---|---|
| `model` | required | the schema class: a `dataclasses.dataclass`, a Pydantic model, a Pydantic dataclass — see [What a schema may be](types.md#what-a-schema-may-be) — or [`Values`](#values), which is no schema at all |
| `key` | required | the section this configuration reads (`[db]` in a TOML file). It also names the environment prefix, the cache entry and every diagnostic; `""` is a configuration with nothing to call itself, which goes with `whole_document()` |
| `executor` | `None` | which pool runs the blocking half of the async calls; `None` follows [`set_executor`](#set_executorexecutor) |
| `secrets` | `()` | dotted paths whose values must never reach a diagnostic. A declared model already says which of its fields are secret and these are *added* to that; for a `Values` configuration they are the only such statement, and the `cache(path, mode)` modes that redact are refused without them |

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
| `whole_document()` | Reads each document as this model's values, with no section header. See [Document Shape](../document-shape.md) |
| `env_file(path)` | A `.env` read as the environment layer, just below the real one |
| `profile_env(variable)` | The variable naming the active profile, for sibling files |
| `cache(path, mode="redacted")` | A last-known-good cache; `redacted`, `full` or `fingerprint` |

### A remote store, written in Python

The eight store crates stay in Rust; the *door* does not. Any object with
`fetch()` and `describe()` is a remote store here — see
[`RemoteSource`](#remotesource).

The same four methods take a **compiled** store from the opt-in second
wheel (`pip install dynamic-config-py[remote]`): `Etcd(...)` and
`Vault(...)` from `dynamic_config.remote` are `RemoteSource`
implementations like any other, and their API is
[on its own page](remote-wheel.md#api) because they are a separate
distribution.

| Synchronous | Async twin | Does |
|---|---|---|
| `remote(source)` | — | Installs the store. Chains; fetches nothing. Allowed **after** the first load too, unlike the source methods |
| `refresh_remote()` | `refresh_remote_async()` | Reads the store and keeps the document, for the next load |
| `clear_remote()` | — | Drops the fetched document; the source stays installed |
| `remote_description` | — | What the installed store's `describe()` said, or `None` |

Fetching is explicit, exactly as it is in Rust: a load merges the document
that was last fetched and touches no network. The remote layer sits above
the files and below the environment.

```python
class OurService(RemoteSource):
    def fetch(self):
        return httpx.get(URL, timeout=5).text, Format.JSON

    def describe(self):
        return "our service"

config = DynamicConfig(Database, key="db").remote(OurService())
config.refresh_remote()
config.init()
```

Three things a caller has to know, each of which has a test:

- **The timeout is the `fetch()` implementation's.** Nothing on the Rust
  side can interrupt Python that has decided not to return, so the
  deadline belongs to the client the method calls.
- **A raising `fetch()` is reported, not fatal.** It arrives as
  `RemoteError` — or `AuthError`, if that is what was raised — with the
  original attached as `__cause__`. Its *message* is deliberately not
  repeated: a store's exception routinely carries the URL it called. The
  previous document and the previous model both keep serving.
- **A `fetch()` may read its own configuration** — `current()`,
  `snapshot()`, `explain()` — because no lock is held across it. The one
  thing it may not do is call `refresh_remote()`, which is refused by
  name rather than left to recurse.

`describe()` is asked **once**, when the source is installed, because the
engine reads it on the load path and a load must not re-enter Python.

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
| `overrides(**values)` | A `with` block that pins those values and restores the previous layer after it — see [Testing](#testing) |
| `clear_defaults()` / `clear_overrides()` / `clear_assignments()` | Empty one layer |
| `alias(old, new)` | Keeps files written before a rename working |
| `bind_env(path, variable)` | Maps one field to one variable by name — `PORT`, `DATABASE_URL` |

These take effect on the next load, so a `set_override` after `init()`
wants a `reload()` behind it. `overrides(...)` is the exception, and that
is the whole reason it exists: it reloads on entry and again on exit.

### Diagnostics

| Method | Returns |
|---|---|
| `source_of(path)` | [`Origin`](#origin) — which layer would supply it — or `None` |
| `is_set(path)` | Whether anything supplies it |
| `explain(path)` | [`Explanation`](#explanation) — every layer's answer, secrets redacted |
| `check()` | [`Report`](#report) — would it load, and is anything unknown |
| `snapshot()` | [`Snapshot`](#snapshot) — the resolved section as data |

### Telemetry

| Method | Returns |
|---|---|
| `status()` | [`ConfigStatus`](telemetry.md#configstatus) — generation, staleness, the last reason, the failure streak |
| `remote_status()` | [`RemoteStatus`](telemetry.md#remotestatus) — fetches, staleness, `reachable`, the failure streak |

Both are a handful of atomic loads: no source is re-read and nothing
blocks, which is what makes them cheap enough for a scrape. `Exposition`
renders either as Prometheus text — see [Telemetry](telemetry.md).

### Properties

| | |
|---|---|
| `key` | The section key |
| `model` | The Pydantic class |
| `generation` | How many models have been installed; zero before the first |

`repr(config)` is those three and nothing else —
`<DynamicConfig Database key='db' generation=3>` — which is what a
debugger session wants and what a log line can survive: shape, never
values, `generation=0` meaning nothing has installed yet.

## Testing

### `overrides(**values)`

The override layer, scoped to a `with` block:

```python
with config.overrides(pool_size=1, host="localhost"):
    ...        # reloaded on entry, with those values pinned
               # the previous overrides are restored and reloaded on exit
```

The long hand is `set_override`, `reload`, `clear_overrides`, `reload` —
four lines whose last two are easy to forget, and forgetting them leaks
into the next test through whatever configuration the module built.

- **Restores rather than clears.** The exit puts back the layer the block
  *found*, so a nested `with` composes and an override set before the
  block still stands after it.
- **Restores on an exception too.** A failing assertion inside the block
  does not decide what the next test sees.
- **`__` is a dot**, the same nesting rule the environment layer uses:
  `pool__max_size=1` means `pool.max_size`. A field whose own name
  contains `__` cannot be spelled this way — use `set_override`.
- **With no arguments** it pins nothing and still restores, which wraps a
  block that calls `set_override` itself.

### The pytest plugin

The package ships one, and pytest finds it through a `pytest11` entry
point — installing `dynamic-config-py` is the whole setup:

```python
def test_the_service_reads_its_file(dynamic_config_workspace):
    (dynamic_config_workspace / "app.toml").write_text('[db]\nport = 5432\n')
    config = DynamicConfig(Database, key="db").file("app.toml")

    assert config.init_and_current().port == 5432
```

| Fixture | Is |
|---|---|
| `dynamic_config_workspace` | A `tmp_path` that is also the working directory, so `file("app.toml")` finds *this* test's copy |
| `dynamic_config_env` | A factory: `dynamic_config_env("APP_")` unsets every variable with that prefix for the test |

Nothing is autouse — a plugin that arrives with the wheel should not
change what a test sees until the test asks. The environment one is
usually wanted for every test, which is one fixture in your own
`conftest.py`:

```python
@pytest.fixture(autouse=True)
def _clean_environment(dynamic_config_env):
    dynamic_config_env("APP_")
```

A suite that turns entry-point discovery off — CI images increasingly set
`PYTEST_DISABLE_PLUGIN_AUTOLOAD` — asks for it by name instead:
`-p dynamic_config.pytest`, on the command line or in `addopts`.

`dynamic_config.pytest` imports pytest and the standard library and
nothing else — not Pydantic, and not the rest of this package's public
surface. It is auto-loaded in every pytest run of every environment the
package is installed in, so a dependency there would be a dependency for
all of them; the binding's own suite runs on these two fixtures, and a
subprocess test imports the module with Pydantic made unimportable.

## Module functions

### `__version__` and `__engine_version__`

The wheel's version, and the version of the `dynamic-config` crate
compiled into it. The two move independently — the Python package
versions on its own schedule — so a bug report can name both.

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

### `Values`

A configuration with **no schema class**: pass `Values` where a model
goes, and every load hands back one of these — a `Mapping` read by dotted
path. See [`Values`: a configuration with no
schema](types.md#values-a-configuration-with-no-schema) for what it gives
up, and the [schemaless chapter](../schemaless.md) for the Rust half.

| Member | Answers |
|---|---|
| `values[path]` | the value at a dotted path, or `KeyError` |
| `values.get(path, default=None)` | the same, with a default |
| `path in values` | whether anything is there |
| `len(values)`, `iter(values)` | the **top-level** keys |
| `values.to_dict()` | a plain `dict` of the whole configuration |
| `values.leaf_paths()` | every dotted path that holds a value, sorted |
| `repr(values)` | the keys, never a value |

### `changed_paths(previous, current)`

Which paths differ between two models (or mappings), as
[`Change`](#change) values. Paths only, never values — including for
secrets, whose values are compared but never reported.

### `@dynamic_config(...)`

Attaches a configuration to a model class and returns the class.

Every argument is keyword-only, and every one of them is one fluent call
on the configuration it builds — the decorator is the
declaration-shaped spelling, not a second set of behaviour.

| Argument | Default | The call it makes | Meaning |
|---|---|---|---|
| `key` | required | `DynamicConfig(model, key)` | The section key: which top-level table is this model's. Also names the environment prefix, the cache entry and every diagnostic. `""` for a configuration with nothing to call itself |
| `files` | `()` | `.file(path)`, once each | Files to merge, in order — later wins, a missing one is skipped |
| `discover` | `None` | `.discover(name, paths)` | `(name, paths)`: look for `{name}.{ext}` in each directory, below the listed files |
| `env` | `None` | `.env(prefix)` | The environment prefix, trailing underscore included |
| `nest` | `None` | `.nest(separator)` | What means nesting inside a variable name; `__` unless given |
| `allow_empty_env` | `False` | `.allow_empty_env()` | Treat `FOO=` as set-to-empty rather than unset |
| `strict_env` | `False` | `.strict_env()` | Refuse ambiguous spellings — `off`, `no`, `nil` |
| `whole_document` | `False` | `.whole_document()` | The documents carry **no section header**: each one *is* this model's values. See [Document Shape](../document-shape.md) |
| `env_files` | `()` | `.env_file(path)` | `.env` files, read as the environment layer and below the real one |
| `profile_env` | `None` | `.profile_env(variable)` | The variable naming the active profile, for sibling files |
| `cache` / `cache_mode` | `None` / `"redacted"` | `.cache(path, mode)` | Last-known-good cache; `redacted`, `full` or `fingerprint` |
| `init` | `False` | `.init()` | Load at decoration — off, because import time is not load time |
| `watch` | `None` | `.watch(debounce).detach()` | Start a detached watcher with this debounce. It does not load: pair it with `init=True` |

It attaches `config`, `current`, `try_current`, `reload`, `source_of` and
`explain` to the class, and refuses a model that declares a field with
one of those names.

`examples/21_decorator_whole_document.py` runs every row of that table,
and shows `whole_document=True` against a file with no header.

### `Configured`

The mixin that makes those six visible to a type checker and to an
editor — `class Database(Configured, BaseModel)`. Runtime behaviour is
unchanged; what changes is that `Database.current()` is typed as
`Database` rather than being an `attr-defined` error. See
[the decorator](../python.md#the-decorator).

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
(tuple of `UnknownKey`: `path`, `suggestion`), `failure`,
`unknown_checked`, and the `is_clean` property. `str(report)` renders the
table the Rust crate prints — paths and origins, never values.

`unknown_checked` is `False` when there was no field list to compare a
document against, which is a [`Values`](#values) configuration: an empty
`unknown` from one is not an all-clear, and the rendering says
`unknown keys: not checked (no field list)` rather than letting it read
as one.

### `Snapshot`

`to_dict()`, `source_of(path)`, `contains(path)`, `leaf_paths()`,
`top_level_keys()`, `is_empty()`, `diff(other)` → `Change` values.

### `Change`

`path` and `kind` (`added`, `removed`, `changed`).

### `ConfigStatus`, `RemoteStatus`, `Failure`

What `status()` and `remote_status()` hand back, and the failure either
may carry. Frozen dataclasses of counts, durations and fixed enums —
never a value, never a store address. Field by field in
[Telemetry](telemetry.md).

### `Exposition`

One or more configurations' status as a Prometheus text body:
`Exposition().add(name, config).add_remote(name, config).render()`, plus
`add_with`/`add_remote_with` for labels of your own. Built per scrape and
thrown away. The metric names are API; see [Telemetry](telemetry.md).

### `RemoteSource`

The ABC a store written in Python subclasses. Two abstract methods, so a
class missing one cannot be instantiated at all — a `TypeError` where the
store is constructed, rather than something a deployment discovers at its
first refresh:

| Method | Answers |
|---|---|
| `fetch()` | `(document, format)` — the text, and the [`Format`](#format) it is written in. Raise to report a failure |
| `describe()` | The store's name, for provenance and error messages. Asked once, at install |

Name the store, never the credential that reaches it: `describe()` is
what `source_of(...)` reports and what every remote error carries.

### `Format`

`Format.JSON`, `Format.TOML`, `Format.YAML` — a `str` enum, so a plain
`"json"` is accepted too.

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
| `RemoteError` | A remote store could not be read — unreachable, refusing, malformed |
| `AuthError` | A credential was rejected, or could not be obtained. Distinct from `RemoteError` on purpose: waiting fixes one and not the other |
| `DecryptError` | An encrypted source could not be decrypted |
| `BackendError` | The engine refused — a source added after loading, for instance |
| `NotInitialisedError` | `current()` before the first successful load |

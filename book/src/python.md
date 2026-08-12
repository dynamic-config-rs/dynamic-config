# Python Bindings

`dynamic-config-py` pairs this engine with the schema you already write:
**Rust resolves, your schema validates, Python reads a cache.**

```sh
pip install dynamic-config-py                     # the import is `dynamic_config`
pip install dynamic-config-py[pydantic]           # + Pydantic models
pip install dynamic-config-py[pydantic-settings]  # + BaseSettings classes
pip install dynamic-config-py[all]                # all of it
```

The base install has **no dependencies**: the engine is compiled into the
wheel, and a `dataclasses.dataclass` is a schema here. Pydantic is an
extra because it is a choice — see
[What a schema may be](python/types.md#what-a-schema-may-be) for what
each kind validates.

```python
from dataclasses import dataclass
from dynamic_config import DynamicConfig

@dataclass
class Database:
    host: str = "localhost"
    port: int = 5432

db = (
    DynamicConfig(Database, key="db")
    .file("config.toml")
    .env("APP_")
    .init_and_current()    # a Database instance — cached, not re-validated
)
```

Everything the Rust side does with sources happens here unchanged: files
merge in call order, the environment beats them, `.env` sits just below
the real environment, profiles overlay sibling files, discovery sits
below listed files, and the runtime layers bracket the rest. The
[precedence chapter](sources-and-precedence.md) is the contract for both
languages.

## Where validation happens, and why it matters

```text
reload trigger (watcher / reload() / init())
    → Rust: load, merge, strict checks          no GIL
    → Rust: resolved tree → dict                GIL, microseconds
    → Python: Model.model_validate(dict)        GIL, once per reload
    → ok:  swap the cached model, wake readers, run hooks
    → err: nothing installs — the previous model keeps serving
```

Validation is the engine's own `validate` hook, which the loader calls
**before** it installs anything. That placement is the whole design:

- **A reader never pays.** `current()` returns the cached instance —
  no boundary crossing, no validation, no lock a writer holds.
- **A reload Pydantic rejects changes nothing.** The previous model keeps
  serving, the generation does not move, and the last-known-good cache is
  not written — exactly what a Rust `validate` refusal does.
- **Validation runs once per resolve**, not once per read and not twice
  per reload.

## The lifecycle

```python
config.init()                          # load, validate, install
candidate = config.load()              # validate only — installs nothing
config.reload()                        # again, on demand

watch = config.watch(debounce=0.25)    # and on every file change
watch.stop()                           # or use it as a context manager

config.current()                       # the model; raises before the first load
config.try_current()                   # or None
config.replace(Database(host="x"))     # install one you built
```

Every call that touches the sources has an async twin —
`init_async`, `load_async`, `reload_async`, `changed_async` — and
[API Reference](python/reference.md) is the full list, with each pair on
one row.

`current()` has none, deliberately: the model is cached on the
configuration object, so reading it is an attribute lookup that needs no
`await` on a loop and blocks nothing on a thread.

`watch(poll_interval=...)` chooses polling over the platform's
notification backend — what network and overlay filesystems need, where
the native watch registers successfully and then silently never fires.
One watcher per configuration object; two configurations of the same
model watch side by side.

### Reacting to a reload

```python
@config.on_change("pool_size")         # only when that path moved
def resize(old, new):
    pool.resize(new.pool_size)

async for db in config.changes():      # any event loop, no callback
    pool.resize(db.pool_size)
```

Hooks run on whichever thread performed the reload, so keep them short:
compare, then *signal* the subsystem that owns the resource — the
[reload lifecycle](reload-lifecycle.md) chapter is the same argument in
Rust. A hook that raises is reported through Python's unraisable channel
and the remaining hooks still run.

[Callbacks](python/callbacks.md) is the whole surface: what `old` and
`new` mean, why a read inside a hook already sees the new model, the
filter above, the scoped `with config.on_reload(...)` form, and how to
hand work to the thread that owns the resource.

`changes()` is an async iterator whose wait happens on a worker thread
with the GIL released, so it drives on asyncio, uvloop, trio's asyncio
compatibility layer — anything. There is a blocking `changed(timeout=…)`
for threads and an awaitable `changed_async(timeout=…)` for a single
shot. [Async & asyncio](python/async.md) is the whole story: which calls
block, which thread each piece runs on, and how cancellation behaves.

## Secrets are derived, not re-declared

```python
class Database(BaseModel):
    host: str
    password: SecretStr        # this is the declaration
```

At construction the binding walks `model_fields` for `SecretStr` and
`SecretBytes` — through `Optional`, unions, containers, nested models,
Pydantic dataclasses and `RootModel`, as dotted paths — and seeds the
same secret list the generated Rust `builder()` seeds. A field is listed
under **every** name a file could carry it under, its aliases included,
because a secret spelled the other way is still a secret
([Aliases](python/types.md#aliases-in-all-four-shapes)). Everything
downstream follows from it: the redacted
[last-known-good cache](persistence.md#last-known-good) drops those
fields, `explain` prints them as `***`, and Pydantic's `ValidationError`
— which by default echoes the offending input — is scrubbed to
locations, messages and error types before it crosses the boundary.

Nobody keeps a second list in step with the first, because there is no
second list.

## What a read costs

The claim is that reading configuration is an attribute lookup. Measured
with `python benchmarks/read_path.py`, which prints the machine it ran on
above the numbers — because a nanosecond figure without one is not a
measurement:

```text
  cpu         Intel(R) Core(TM) i7-14700F
  cores       28
  memory      126 GiB
  os          Linux 7.1.4
  python      CPython 3.14.6 (release)
  rounds      200000 per measurement
```

| | ns per read | against a global |
|---|---|---|
| `config.current()` | 29 | 1.1× |
| a module global | 27 | — |
| `Model.model_validate(dict)` | 990 | 34× |

`current()` is within a tenth of a bare global because it *is* a Python
attribute: the engine publishes each new model into the configuration
object as it installs, so a read never crosses back into Rust. The third
row is the number that matters — it is what every read would cost if the
model were validated per read instead of once per reload.

The **ratios** are what travels between machines; the nanoseconds belong
to that block above them. A slower laptop moves all three numbers and
leaves the two ratios where they are, which is why the argument is made
with ratios.

## Diagnostics

```python
config.source_of("port")       # Origin(kind='env', detail='APP_DB_PORT')
config.is_set("pool.size")     # False
print(config.explain("port"))  # every layer's answer, as a table
report = config.check()        # would it load? any unknown keys?
config.snapshot().to_dict()    # the resolved section, as data
```

The rules the [diagnostics chapter](validation-diagnostics.md) states hold
here too: paths, never values — except `explain`, which is the one
diagnostic whose job is values, and which redacts the secret ones. Every
`repr()` in the binding shows shape rather than content, so an object
landing in a log line cannot leak a configuration.

`changed_paths` is the audit half of a reload — what *moved*, without
what it moved to:

```python
from dynamic_config import changed_paths

config.on_reload(
    lambda old, new: log.info("configuration changed: %s",
                              ", ".join(str(c) for c in changed_paths(old, new)))
)
```

It compares the real values — including secrets, because comparing the
mask Pydantic renders would make two different passwords look identical
and miss the one change most worth noticing — and reports only paths and
whether each was added, removed or changed.

## Errors

One hierarchy, mirroring `ErrorKind`:

```python
from dynamic_config import DynamicConfigError, InvalidError, MissingError

try:
    config.init()
except InvalidError as error:
    for report in error.errors:       # Pydantic's own report, scrubbed
        print(report["loc"], report["msg"], report["type"])
except DynamicConfigError as error:
    print(error.kind, error.path, error.origin)
```

`DynamicConfigError` catches everything; each instance carries `kind`,
`path`, `origin_kind` and `origin` so a program can branch without
parsing English.

## The decorator

```python
from dynamic_config import dynamic_config

@dynamic_config(key="db", files=["config.toml"], env="APP_")
class Database(BaseModel):
    host: str
    port: int = 5432

Database.config.init()
Database.current()
```

Sugar over the same object: the decorator builds a `DynamicConfig`,
stores it as `Model.config` and attaches `current`/`try_current`/
`reload`/`source_of`/`explain` classmethods. It does **not** load at
import time — reading files while a module is being imported is a
surprise nobody asked for; pass `init=True` when a script wants exactly
that. Decorating one class twice is an error, mirroring the crate's
one-configuration-per-type rule.

In Python a runtime-configured decorator is idiomatic where Rust's
argument-free attribute was not. The engine-level rule holds in both:
declaration is separate from the configurable builder underneath.

## Typing

`DynamicConfig` is `Generic[M]`, so `current()` comes back as *your*
model rather than as `Any`:

```python
config = DynamicConfig(Database, key="db")
reveal_type(config.current())          # Database
```

Stubs ship with the package and `mypy --strict` runs over the facade in
CI, so this stays true.

## With a web framework

```python
@app.get("/health")
async def health() -> dict[str, str]:
    db = config.current()       # once per request; reuse the value
    return {"host": db.host}
```

Read `current()` once per request and use that value, exactly as the Rust
guidance says: a reload landing mid-request would otherwise show one
request two configurations. [Web Frameworks](python/frameworks.md) has
the FastAPI, Flask and Django patterns in full, including what not to do
in each (copying into `app.config`, freezing into Django's settings), and
the pre-forking-server caveats.

## Data types

Whatever Pydantic validates, this loads: enums, `datetime`, `UUID`,
`Decimal`, paths, addresses, containers, nested models, unions,
`SecretStr`. [Data Types](python/types.md) covers the whole range and the
three conversions that have to be exact — an integer staying an integer,
a bool not becoming `1`, and a large `u64` keeping its digits.

And whatever a *model* may be, it may be the schema: inheritance,
mixins, `model_config`, validators, computed fields, `RootModel`,
Pydantic dataclasses, generics, discriminated unions — and
`pydantic_settings.BaseSettings`, whose own sourcing declaration
`DynamicConfig.from_settings(...)` translates into engine sources so an
existing settings class keeps the variable names its deployment already
sets. [What a schema may be](python/types.md#what-a-schema-may-be) and
[pydantic-settings](python/types.md#pydantic-settings).

## What is not exposed, and why

| Not exposed | Why |
|---|---|
| The [remote stores](remote-stores.md) | their clients — gRPC, the AWS SDK, three HTTP stacks — would ride into every wheel |
| `RemoteSource` implemented in Python | a Python object on the fetch path deserves its own design pass |
| Encrypted files | decryption needs a `Decryptor`, which is a Rust trait; decrypt with the [CLI](cli.md) and point this at the result |
| `save`, JSON Schema | Pydantic already does both, better |

The first two are on the roadmap; the last two are not.
[Limitations](python/limitations.md) has the full list with the reasoning
— including the constraints that are not omissions at all: sources fixed
after the first load, one watcher per configuration, and why
`ValidationError` is rebuilt rather than re-raised.

## Examples

Sixteen runnable scripts ship with the package —
[`examples/`](https://github.com/ctolon/dynamic-config/tree/main/dynamic-config-python/examples),
from the twenty-line quick start to multi-tenant configuration, the
diagnostics tour, several configurations on one event loop (as values
and as decorated classes), every callback shape, an existing
`pydantic-settings` class, and the three framework integrations. None needs a server or a setup step,
and all of them run in CI, because an example nobody runs is
documentation that has already started rotting — and the framework ones
are driven again by the integration suite, which asserts what they
answer rather than only that they exit zero.

## How it is built

[Implementation Details](python/internals.md) is the inside view: where
validation is hooked and why that placement is the whole design, how a
validated model is published exactly once, why the read path never
crosses back into Rust, what the GIL and thread rules are, and what the
two changes the Rust crate needed were.

## Interpreter shutdown

A watcher thread that outlives finalization would call into a Python that
is no longer there — the classic embedding crash. The binding registers
an `atexit` handler that stops every watcher and drops every cached model
while the interpreter is still whole, and the test suite exercises exactly
that: a process exiting mid-reload-storm with a detached watcher running.

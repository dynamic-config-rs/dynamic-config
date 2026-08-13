# Limitations

What the Python bindings deliberately do not do, and why. As with the
Rust crate's [Limitations](../limitations.md), the list exists so that a
missing feature reads as a decision rather than an oversight — and so
that anyone who disagrees can argue with the reason instead of guessing
at one.

## Not exposed

### Remote stores, in the base wheel

etcd, Consul, Vault, NATS, Redis, S3, Firestore and git stay out of the
*ordinary* wheel. Their clients are a gRPC stack, the AWS SDK, a git
implementation and three HTTP clients between them, and a wheel is built
per platform — every one of those dependencies would ride into every
install, including the ones reading a single TOML file.

Two things are exposed instead, and between them they cover most of it.

The **door**: [`RemoteSource`](remote-stores.md) is implementable in
Python, so a store with no Rust client — a company's own service, a file
a sidecar writes — is a class with `fetch()` and `describe()`. That is in
the base wheel and needs nothing extra.

The **stores**, as an opt-in second wheel:
[`pip install dynamic-config-py[remote]`](remote-wheel.md) buys all eight
Rust clients — etcd, Consul, Vault, NATS, Redis, S3, Firestore and git —
compiled, imported as `dynamic_config.remote`. An extra cannot turn on a
Cargo feature in a binary that was compiled weeks ago on a release
runner, so it resolves to a distribution of its own; the base install is
unchanged, and importing `dynamic_config.remote` without it raises an
`ImportError` naming the extra.

**Custom proxies** are not exposed for any of them, and the reason is
structural rather than an omission: each Rust builder reaches one by
taking its client's own configuration type — an `etcd_client::ConnectOptions`,
a `ureq::Agent`, an `SdkConfig` — which exists precisely so that options
this project has never heard of keep working, and which has no Python
spelling. Nor is `watch()`: a Rust callback on a Rust thread calling into
Python is a second GIL story, and `refresh_remote()` on a timer is what
Python has instead. That one costs more for **git** than for the others,
and the wheel says so rather than leaving it to be discovered: git is the
only store whose multi-file source can be watched in Rust, because one
fetch resolves one commit — so it is the only place where the missing
`watch()` costs a capability rather than a convenience.

**TLS is exposed**, and it is the counter-example worth reading before
adding to this list. A private certificate authority and a client
certificate are the settings a hardened deployment actually needs, and
they reach Python because the Rust surface for them was built as
[**data**](remote-wheel.md#tls-a-private-authority-and-a-client-certificate)
— paths and PEM bytes, with no client type in any signature — rather than
as another door onto a client's own type. Two stores cannot express all of
it and **refuse the part they cannot** rather than ignoring it: `Nats`
takes certificate paths and not PEM bytes, because `async-nats` opens the
files itself, and `S3` takes no client certificate at all, because the AWS
SDK's TLS context is a trust store with no slot for one. Both raise at
construction, naming the call and the way out — a caller who believes they
pinned an authority and did not is worse off than one whose program will
not start.

### Encrypted files

`encrypted_file(...)` needs a `Decryptor` implementation, which is a Rust
trait. Shipping `age` to make one usable would put a crypto stack in
every wheel for a door only Rust can open. Decrypt with the
[CLI](../cli.md) or your deployment's own tooling and point this at the
result.

### `save` and JSON Schema

Pydantic already serializes models and emits JSON Schema, and does both
better than a second implementation would. `model_dump_json()` and
`model_json_schema()` are the answers.

## Constraints worth knowing

### Sources are fixed after the first load

`.file(...)`, `.env(...)` and the rest raise once anything has loaded.
Sources are how a configuration is *identified*; changing them makes it a
different configuration, and pretending otherwise would leave the watcher
watching one thing and the loader reading another. Build a second
`DynamicConfig`.

### One watcher per configuration object

A second `watch()` on the same object raises `AlreadyExists`, exactly as
the Rust engine does — a second watcher could only mislead. Two
`DynamicConfig` objects over the same model watch side by side without
interfering, which is what
[multi-tenant](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/06_multi_tenant.py)
uses.

### `validate` is Pydantic's, not a second hook

There is no `.validate(fn)` on the Python builder, because the model
already has `field_validator` and `model_validator`. A rejection there
behaves exactly as a Rust `validate` refusal: nothing installs, the cache
is not written, the previous model keeps serving.

### `ValidationError` does not pass through untouched

Pydantic's `str()` embeds `input_value=...`, which would put the
offending configuration value into every log line that caught it. A
rejection raises `InvalidError` instead, whose message is the scrubbed
rendering and whose `.errors` is Pydantic's own report with the input
values removed. The locations, messages and error types are all there.

### There is no `pip install dynamic-config-py[tokio]`

The Rust crate has a `tokio` feature, and it is reasonable to expect the
wheel to expose the same switch. It does not, and the reason has changed
shape now that [the remote wheel](remote-wheel.md) exists — so this
section says both halves.

**A wheel is already compiled.** A pip extra installs *Python*
distributions; it cannot turn on a Cargo feature in a binary that was
built weeks ago on a release runner. Anything that needs a different
build has to be a second wheel. That much was always true, and it is
exactly what `dynamic-config-py[remote]` turned out to be.

**Nothing in the base wheel awaits a tokio task.** The Rust `tokio`
feature routes the crate's *own* async loads into tokio's blocking pool.
The base binding never takes that path: Python's event loop can await a
Python future and nothing else, so the blocking half goes to a Python
executor and the result comes back as a Python object. Enabling `tokio`
there would add a runtime to every wheel that no code in it would enter —
which is why the base wheel still refuses the feature, and why a
`[tokio]` extra would be a distribution differing in a way nobody could
observe.

**The remote wheel is where a runtime finally means something**, and it
owns one rather than turning the engine's feature on. etcd's client is
async, so something has to drive its `fetch`; that is one runtime, two
worker threads, started when the first store that needs one is
constructed and never shut down. The
[whole story](remote-wheel.md#the-tokio-runtime) — including what happens
if the calling thread is already inside somebody else's runtime, and why
an immortal runtime is safe here — is on that page.

Note what the remote wheel does *not* do: it does not enable
`dynamic-config/tokio`. That feature is about where the **engine's** own
async loads go, and the engine in these wheels does no async loading at
all. The runtime exists for the store client and for nothing else.

`set_executor` is unchanged and still answers the question it always
answered — *which pool pays for the blocking work*:

```python
dynamic_config.set_executor(ThreadPoolExecutor(2, thread_name_prefix="config"))
```

`refresh_remote_async()` runs the whole fetch on that pool, remote wheel
or not. With the remote wheel installed, the tokio runtime is what the
fetch uses once it arrives there; the two are stacked, not competing.

### Free-threaded CPython is one interpreter and one platform

The module declares `Py_mod_gil = Py_MOD_GIL_NOT_USED`, the suite runs on
a real 3.14 free-threading build, and the audit behind that is
[a page of its own](free-threading.md). What the claim rests on is
narrower than the claim sounds: one interpreter version, one platform,
and ten repeated runs of the threading and shutdown suites — evidence
rather than proof. The free-threaded wheels are manylinux `x86_64` and
`aarch64` only, so macOS and Windows on a `t` interpreter build from
source. `cp313t` does not exist at all: PyO3 0.29 dropped it when CPython
promoted free-threading from experimental to supported in 3.14.

### A Python `fetch()` cannot be timed out from outside

A [remote store written in Python](remote-stores.md) runs as ordinary
Python on the thread that asked for the refresh, and nothing on the Rust
side can interrupt Python that has decided not to return. A worker thread
and a deadline would let `refresh_remote()` give up while the fetch kept
running, which is an error message rather than a cure. The deadline
belongs to the client `fetch()` calls — `httpx.get(..., timeout=5)` — and
`Ctrl-C` still works, because a `KeyboardInterrupt` out of a fetch
propagates unchanged.

### Creating configurations in a loop leaks a little

Each `DynamicConfig` allocates the runtime layers the engine takes as
`&'static` — a few hundred bytes, once, per configuration object, never
per reload. A program with a handful of configurations pays nothing worth
measuring; a program constructing thousands in a loop is doing something
the design did not anticipate, and should hold one and use
`set_override` instead.

### The decorator does not load at import time

`@dynamic_config(...)` attaches a configuration and stops. Reading files
while a module is being imported is a side effect nobody asked for, and
it makes import order load-bearing. Call `Model.config.init()` where your
program starts, or pass `init=True` if you are writing a script and want
exactly that.

## Versioning

The Python package versions **independently of the Rust crates**. The ten
crates on crates.io move in lockstep because they pin each other exactly;
the wheel has no such tie — it embeds the engine rather than depending on
a published version of it — so bumping it for a Rust-only fix would ask
every Python user to upgrade for a release with nothing in it for them.

It moves when the Python package changes: a new API, a behaviour change,
or an engine bump worth shipping. `dynamic_config.__version__` and
`pip show dynamic-config-py` report that number; the engine's own version
is what the wheel was built against and is recorded in the changelog
entry that shipped it.

## What a dataclass schema does not do

The dependency-free schema validates structurally and does not coerce.
Three exceptions aside — an `Enum` takes its member's value,
`date`/`time`/`datetime` parse through `fromisoformat`, and a type that
builds from a single argument is built from it — a value whose type does
not match its annotation is a validation failure rather than an
assignment. If you want a string parsed into something the stdlib cannot
parse it into, constraints, aliases, or validators, that is what
`pip install dynamic-config-py[pydantic]` buys.

One limitation there is Python's rather than this library's: annotations
are resolved with `typing.get_type_hints`, which looks in the module
where the class was defined. A dataclass declared *inside a function*
names types that module cannot see, so its annotations stay strings and
there is nothing to check them against — the fields are filled without a
type check. Declare configuration dataclasses at module level. Pydantic
meets the same wall and answers it with `model_rebuild()`.

## Not planned

- **A settings-source shim for `pydantic-settings`.** The two libraries
  answer the same question differently; wiring this in as a
  `PydanticBaseSettingsSource` would inherit that library's lifecycle
  (read once, at construction) and lose the reloading that is the whole
  point. Support went the other way instead — a settings class is a
  schema here, and `from_settings` translates its declaration into engine
  sources. See [pydantic-settings](types.md#pydantic-settings).
- **Automatic reload on attribute access.** Reading configuration would
  become an I/O operation with unpredictable latency, which is precisely
  the design this library exists to avoid.
- **A global default configuration.** `dynamic_config.current()` with no
  object would be a singleton by another name — the same thing the Rust
  crate refuses in
  [Not planned](../limitations.md#not-planned).

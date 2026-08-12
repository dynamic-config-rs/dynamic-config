# Limitations

What the Python bindings deliberately do not do, and why. As with the
Rust crate's [Limitations](../limitations.md), the list exists so that a
missing feature reads as a decision rather than an oversight — and so
that anyone who disagrees can argue with the reason instead of guessing
at one.

## Not exposed

### Remote stores

etcd, Consul, Vault, NATS, Redis, S3 and Firestore stay in Rust. Their
clients are a gRPC stack, the AWS SDK and three HTTP clients between
them, and a wheel is built per platform — every one of those
dependencies would ride into every wheel for every user, including the
ones reading a single TOML file.

If you need one from Python today, the shape that works is a small Rust
service or the [CLI](../cli.md) writing what it fetched to a file this
library watches. An opt-in `dynamic-config-py[etcd]` extra with its own
wheel is on the roadmap; the reason it is not here yet is that nobody has
asked, and speculative wheels are a maintenance burden with no user.

### `RemoteSource` implemented in Python

Writing a store *in Python* would put a Python object on the fetch path,
which means the GIL is held while a network call happens, and a fetch
that raises has to be turned into a Rust error somewhere sensible. Both
are solvable; neither is solvable *casually*, and a half-done version of
this is worse than none.

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
wheel to expose the same switch. It cannot, for two reasons that stack:

**A wheel is already compiled.** A pip extra installs *Python*
distributions; it cannot turn on a Cargo feature in a binary that was
built weeks ago on a release runner. A tokio-enabled wheel would have to
be a second wheel — a separate distribution name or a build tag — and
shipping two wheels that differ in a way nobody can observe is worse than
shipping one.

**Nothing here awaits a tokio task.** The Rust `tokio` feature routes the
crate's *own* async loads into tokio's blocking pool. This binding never
takes that path: Python's event loop can await a Python future and
nothing else, so the blocking half goes to a Python executor and the
result comes back as a Python object. Enabling `tokio` would add a
runtime to every wheel that no code in it would enter.

What actually answers the same question — *which pool pays for the
blocking work* — is [`set_executor`](reference.md#set_executorexecutor):

```python
dynamic_config.set_executor(ThreadPoolExecutor(2, thread_name_prefix="config"))
```

The one thing that would make a tokio build meaningful is the async
remote stores (etcd, NATS, S3), whose clients are tokio-based. That is
the roadmap item to watch; if it lands, it lands as an opt-in wheel with
the runtime it needs, not as a flag on this one.

### Free-threaded CPython is not declared supported

The engine's read path is lock-free and the binding's state sits behind
ordinary locks, so there is no particular reason to expect trouble — but
that is not an audit of the convert-validate-swap step, and declaring
support without one would be a promise made on optimism. Roadmapped as
its own piece of work.

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
- **A `secrets_dir` equivalent.** pydantic-settings can read a directory
  in which each file is one value, which is how Docker and Kubernetes
  mount secrets. The engine has no such source, so `from_settings`
  refuses a class declaring one rather than pretending it worked. It is a
  reasonable source to add later; it is the only translation that had to
  be given up.
- **Automatic reload on attribute access.** Reading configuration would
  become an I/O operation with unpredictable latency, which is precisely
  the design this library exists to avoid.
- **A global default configuration.** `dynamic_config.current()` with no
  object would be a singleton by another name — the same thing the Rust
  crate refuses in
  [Not planned](../limitations.md#not-planned).

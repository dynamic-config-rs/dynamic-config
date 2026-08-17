# Python Bindings

The Python binding has **a book of its own**:

### → [dynamic-config for Python](https://dynamic-config-rs.github.io/python/)

```sh
pip install dynamic-config-py                     # the import is `dynamic_config`
pip install dynamic-config-py[pydantic]           # + Pydantic models
pip install dynamic-config-py[msgspec]            # + msgspec Structs
pip install dynamic-config-py[remote]             # + the Rust etcd and Vault clients
```

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

**Rust resolves, your schema validates, Python reads a cache.** This engine
does the sources, the layering, the profiles, the watcher, the
last-known-good recovery and the provenance; a `dataclasses.dataclass`, a
Pydantic model, a `msgspec.Struct` or `Values` does the validating, once per
successful resolve rather than once per read.

The Python documentation is [its own book](https://dynamic-config-rs.github.io/python/),
on the same site. It links back here for
[precedence](sources-and-precedence.md), [document shape](document-shape.md),
[schemaless configuration](schemaless.md) and [telemetry](telemetry.md) —
engine behaviour, identical in every language.

## What is in it

| Chapter | What it answers |
|---|---|
| [API Reference](https://dynamic-config-rs.github.io/python/reference.html) | Every method, every argument, every default |
| [Callbacks](https://dynamic-config-rs.github.io/python/callbacks.html) | `on_reload`, `on_change`, scoped guards, the thread a hook runs on |
| [Async & asyncio](https://dynamic-config-rs.github.io/python/async.html) | `init_async`, `async for config.changes()`, which pool pays |
| [Data Types](https://dynamic-config-rs.github.io/python/types.html) | What a schema may be, and what each kind validates |
| [Web Frameworks](https://dynamic-config-rs.github.io/python/frameworks.html) | FastAPI, Flask, Django |
| [Telemetry](https://dynamic-config-rs.github.io/python/telemetry.html) | `status()`, and the Prometheus exposition |
| [Remote Stores](https://dynamic-config-rs.github.io/python/remote-stores.html) | A store written in Python, and the second wheel that carries the Rust ones |
| [Implementation Details](https://dynamic-config-rs.github.io/python/internals.html) | What crosses the boundary, and how often |
| [Free-Threaded CPython](https://dynamic-config-rs.github.io/python/free-threading.html) | The `cp314t` wheel, and what was measured |
| [Limitations](https://dynamic-config-rs.github.io/python/limitations.html) | What it will not do, and why |

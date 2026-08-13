# Remote Stores in Python

The eight store crates — etcd, Consul, Vault, NATS, Redis, S3, Firestore
and git — stay in Rust, and [they stay out of the ordinary wheel](limitations.md#remote-stores-in-the-base-wheel):
their clients are a gRPC stack, the AWS SDK, a git implementation and
three HTTP clients between them, and every one of those would ride into
every wheel for every user, including the ones reading a single TOML file.

> All eight now ship compiled, in an opt-in second wheel:
> `pip install dynamic-config-py[remote]`. See
> [Remote Stores in Rust](remote-wheel.md). This page is the other half,
> and it needs no extra: a store *written in Python* is in the base
> wheel, and is the answer for every store that has no Rust client at
> all — a company's own service, a sidecar, an API nobody will write a
> client for — and for the capabilities the wheels do not expose, such as
> a custom proxy or a `watch()` that pushes. TLS is no longer one of
> those: a private certificate authority and a client certificate are
> [`TlsConfig`](remote-wheel.md#tls-a-private-authority-and-a-client-certificate),
> which every store in the second wheel takes.

What ships in the base wheel is the **door**. Any object with `fetch()` and
`describe()` is a remote store here, so a company's own service, a file a
sidecar writes, or an API nobody will ever write a Rust client for needs
no Rust at all:

```python
import httpx
from dynamic_config import DynamicConfig, Format, RemoteSource

class ConfigService(RemoteSource):
    def fetch(self):
        response = httpx.get("https://config.internal/v1/db", timeout=5)
        response.raise_for_status()
        return response.text, Format.JSON

    def describe(self):
        return "the config service"

config = DynamicConfig(Database, key="db").remote(ConfigService())
config.refresh_remote()
config.init()
```

Remote stores in Rust or in Python, the same engine either way — which is
the point of the item: the choice is the user's, and neither answer is a
second-class one.

## Fetching is explicit

```text
refresh_remote()   →  fetch, keep the document
init() / reload()  →  merge the kept document, no I/O
```

The same split the Rust crate makes, for the same reason: configuration
is read on nearly every request, and a network round trip there would be
indefensible. A `fetch()` happens when you ask for one, never on a load.

`config.watch(...)` watches *files*. A store that pushes — a long poll, a
subscription — is a loop you write, calling `refresh_remote()` and
`reload()` when it hears something.

## Where the document lands

```text
defaults < files < remote < environment < flags < overrides
```

Above the files, because centrally distributed configuration should beat
what a package shipped; below the environment, because a machine's own
settings should beat what a central store thinks it wants.

`source_of("port")` answers `Origin(kind='remote', detail=...)`, and the
detail is what `describe()` said.

## The GIL, measured

The concern that made this feature wait: a Python object on the fetch
path means the interpreter is *in* the fetch, and if the GIL were held
for the length of an HTTP request then every other thread in the process
would stop for it.

Measured, that is not what happens. A `fetch()` doing I/O releases the
GIL itself — `socket.recv`, `time.sleep` and every stdlib blocking call
do — so the rest of the process keeps running. Against a second thread
counting in a loop, with a `fetch()` that sleeps 200 ms:

| the fetch is | the other thread runs at |
|---|---|
| I/O-bound (`time.sleep`, a socket) | 68–102% of its free-running rate |
| CPU-bound (a busy loop) | 37–43% — i.e. it *shares*, like any two Python threads |

Neither is a stopped thread, which is what the worry was about, and the
CPU-bound row is the ordinary arithmetic of two Python threads under a
GIL rather than anything this binding does. That measurement is
`tests/test_remote.py::test_a_python_fetch_does_not_stop_other_threads`,
and it is why there is no worker thread behind this API: the indirection
a worker would buy is not needed, and it would have cost a deadlock (see
below).

Everything *around* the Python call releases the GIL properly:
`refresh_remote()` detaches for the whole refresh, and the shim re-takes
it only to call `fetch()`.

## The timeout is yours

Nothing on the Rust side can interrupt Python that has decided not to
return. A worker thread and a channel `recv` with a deadline would let
`refresh_remote()` *give up*, but the abandoned thread would still be
running the fetch, so it buys an error message rather than a cure.

So the contract is the one Python already has: **the deadline belongs to
the client `fetch()` calls.**

```python
def fetch(self):
    return httpx.get(URL, timeout=5).text, Format.JSON
    #                     ^^^^^^^^^ this is the timeout
```

A store with no timeout is a `refresh_remote()` with no timeout. `Ctrl-C`
still works: a `KeyboardInterrupt` raised out of a fetch propagates
unchanged rather than being reinterpreted as a store failure.

## When a fetch fails

Whatever `fetch()` raises arrives as `RemoteError` — or `AuthError`, if
that is what was raised, because *this credential was refused* is worth
telling apart from *the store is unreachable*: waiting fixes one and not
the other.

```python
try:
    config.refresh_remote()
except AuthError:
    stop()          # waiting will not help
except RemoteError as failure:
    log.warning("config store unreachable: %s", failure)
    log.debug("the store said", exc_info=failure.__cause__)
```

Two properties are worth stating plainly, because both are enforced by
tests:

**The exception's message is not repeated.** A store's exception
routinely carries the URL it called, and a URL routinely carries a token
— so the error says which store, and what *type* was raised, and stops
there. The exception itself is attached as `__cause__`, which is where a
traceback and `logging.exception` already look. Values stay out of
diagnostics here as everywhere else.

**Nothing is poisoned.** A failed fetch installs nothing: the document
from the last good fetch is still there, the installed model still
serves, the last-known-good cache is untouched, and the next refresh
works. A store having a bad afternoon is not a configuration failure.

## Reading the configuration from inside a fetch

Allowed, and tested. No lock is held across the fetch, so a `fetch()` may
call `current()`, `try_current()`, `snapshot()`, `explain()` or
`reload()` on the configuration it is fetching for. That is what makes a
source like this one work:

```python
class Incremental(RemoteSource):
    def __init__(self, config):
        self.config = config          # the cycle every real source closes

    def fetch(self):
        since = self.config.current().revision
        return httpx.get(f"{URL}?since={since}").text, Format.JSON
```

The cycle that closes — the configuration holds the source, the source
holds the configuration — collects like any other: the source is an edge
the engine object reports from `tp_traverse` and drops from `tp_clear`.

The one thing a `fetch()` may **not** do is call `refresh_remote()`. That
is the refresh it is answering, and it is refused by name:

```text
BackendError: refresh_remote() was called from inside a remote source's
own fetch(); a fetch must not drive the refresh it is answering.
```

The refusal is per-thread and does not stick.

## `describe()` is asked once

When the source is installed, not per fetch and not per load. The engine
reads `describe()` on the **load** path — it is where a remote value's
provenance comes from — and a load runs with the GIL released, on a
watcher thread as often as on a caller's. Asking Python there would put a
re-entry, and a possible Python exception, on every load of every
configuration that has a store.

So `describe()` should be cheap and constant, and it should name the
store rather than the credential that reaches it: it is rendered in
`source_of`, in `explain`, and in every remote error.

## Swapping and clearing

`remote(source)` may be called again at any time, including after the
first load — unlike `file(...)` and the other source methods, and exactly
as the Rust `set_remote` may. Installing a new source **drops whatever
the previous one had fetched**, because a new store answering with the
old one's values would be a puzzle nobody needs.

`clear_remote()` drops the document and keeps the source.

## Async

`refresh_remote_async()` runs the fetch on a worker thread, so a
`fetch()` written with a blocking client — which is most of them — does
not stall the event loop. Note what it does not do: it does not make
`fetch()` itself awaitable. A source that wants an async client runs its
own loop inside `fetch()`, or fetches on a thread of its own and hands
this one what it has.

## The complete example

[`examples/18_python_remote_source.py`](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/18_python_remote_source.py)
runs all of the above end to end, GIL measurement included, and needs no
server.

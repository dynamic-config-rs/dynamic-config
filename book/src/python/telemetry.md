# Telemetry in Python

Two questions, and they are adjacent rather than the same: **did the
document install**, and **did the store answer**. A service that watches
only the first cannot tell a store that went away from a configuration
nobody has changed.

```python
status = config.status()

if not status.is_healthy:
    log.warning("config has failed %d reloads", status.consecutive_failures)
```

Everything here is a snapshot of a handful of atomic loads: nothing is
re-read, no source is touched, nothing can block. That is what makes it
cheap enough to take on every scrape — and it is the same contract the
Rust [`telemetry`](../telemetry.md) feature makes, through the same
engine.

## `ConfigStatus`

What `config.status()` hands back.

| Field | Type | Means |
|---|---|---|
| `generation` | `int` | Models installed since the process started; zero before the first |
| `stale_for` | `float \| None` | Seconds since the serving model was installed. `None` — not zero — before the first install, because zero reads as *just now* |
| `last_reason` | `str \| None` | Why it was installed: `initial`, `manual`, `file_changed`, `remote`, `replaced` |
| `last_failure` | `Failure \| None` | The most recent reload that installed nothing |
| `consecutive_failures` | `int` | Reloads that installed nothing since one did. **Zero is healthy** |
| `is_healthy` | `bool` | `consecutive_failures == 0`, decided by the engine rather than recomputed here |

`stale_for` is the number most alerts are written against: *this
service's configuration has been stale for an hour* is the page that
matters, and a failing reload leaves the previous model serving — so
staleness is what says so, not an outage.

## `RemoteStatus`

What `config.remote_status()` hands back.

| Field | Type | Means |
|---|---|---|
| `fetches` | `int` | Documents the store handed over, pulled or pushed |
| `stale_for` | `float \| None` | Seconds since the last one arrived |
| `last_fetch_duration` | `float \| None` | How long the last *pulled* fetch took. `None` again after a push: the previous pull's duration beside a push's timestamp would describe neither |
| `last_failure` | `Failure \| None` | The most recent fetch that returned nothing |
| `consecutive_failures` | `int` | Fetches that returned nothing since one did |
| `reachable` | `bool \| None` | **Three states.** `None` before anything has been asked of the store at all — a source installed and never fetched is not *down*, and reporting it as down is how a scrape at startup pages somebody |

It touches no engine, so asking a configuration that has never loaded
does not fix its sources.

## `Failure`

| Field | Type | Means |
|---|---|---|
| `kind` | `str` | The category: `io`, `parse`, `missing`, `type`, `env`, `invalid`, `remote`, `auth`, `decrypt`, `backend` — the names the [exception classes](reference.md#exceptions) carry |
| `path` | `str` | The dotted key path, empty when the failure belongs to the load as a whole |
| `seconds_ago` | `float` | How long before the status was taken it was recorded |

A failure is kept after a later success, because it is history; the
*health* is `consecutive_failures` on the status holding it.

### Why there are no timestamps

The engine records *when* with a monotonic clock, deliberately: those
numbers are read as **how long ago**, and a wall clock stepping backwards
under NTP would make a freshly loaded configuration look stale. A
monotonic instant has no epoch to convert from — not Unix's, and not
`time.monotonic()`'s, which is a different clock read from a different
origin even inside one process.

So what crosses is elapsed seconds, measured when the status was taken.
There is deliberately no `loaded_at` and no `datetime`: building one by
subtracting from `time.time()` would claim a precision the engine
refused to claim, and would be wrong in exactly the case the monotonic
clock was chosen for. A service that needs a wall-clock timestamp takes
one itself in an `on_reload` hook — that is its clock, and its decision.

## `Exposition`: Prometheus text

```python
from dynamic_config import Exposition

@app.get("/metrics")
def metrics() -> Response:
    body = Exposition().add("db", config).add_remote("db", config).render()

    return Response(body, media_type="text/plain; version=0.0.4")
```

Built per scrape and thrown away: every sample comes from a status, so
there is nothing worth keeping between scrapes and nothing to go stale.
This package chooses no metrics ecosystem for the service importing it —
exactly as the Rust crate does not — so what it hands back is a string
for whatever `/metrics` route you already have.

| Method | Effect |
|---|---|
| `add(name, config)` | The configuration's own series, labelled `config="{name}"` |
| `add_with(labels, config)` | The same, with a label mapping of your choosing — an application *and* a profile, say |
| `add_remote(name, config)` | The configuration's remote store, usually under the same name so the two halves join in a query |
| `add_remote_with(labels, config)` | The same, with your labels |
| `render()` | The body. Durations are measured here, so the seconds are as fresh as the response |

Every method except `render()` returns the exposition, so the calls
chain.

### What it emits

Per configuration added:

| Name | Type | Extra label |
|---|---|---|
| `dynamic_config_installs_total` | counter | |
| `dynamic_config_last_success_seconds` | gauge | |
| `dynamic_config_consecutive_failures` | gauge | |
| `dynamic_config_last_failure_seconds` | gauge | |
| `dynamic_config_last_reload_info` | gauge | `reason` |
| `dynamic_config_last_failure_info` | gauge | `kind` |

Per remote source added:

| Name | Type | Extra label |
|---|---|---|
| `dynamic_config_remote_up` | gauge | |
| `dynamic_config_remote_fetches_total` | counter | |
| `dynamic_config_remote_last_fetch_seconds` | gauge | |
| `dynamic_config_remote_last_fetch_duration_seconds` | gauge | |
| `dynamic_config_remote_consecutive_failures` | gauge | |
| `dynamic_config_remote_last_failure_info` | gauge | `kind` |

**These names are API.** They end up in dashboards and alert rules, so a
rename is a breaking change.

A fact that does not exist yet is **absent** rather than zero: no
`last_success_seconds` before the first install, no `remote_up` before
the store has been asked anything. An absent series is a gap in a graph;
a zero is a claim.

### What never becomes a label

No configured value, no key path, no store address. The only string a
source can produce for itself is `describe()`, which is a URL and
routinely carries `user:password@host` — so the name a series carries is
**yours**, passed to `add`, and there is no overload that takes the
store's own. A `Failure`'s dotted path stops at the status object: the
exposition renders the failure's *category* and never its path, which is
unbounded label cardinality as well as a detail nobody asked to publish.

Cardinality is bounded and the bound is the caller's: six series per
configuration per scrape, six more per remote source, with label sets
from fixed enums — five reload reasons, ten error kinds. Label names are
sanitised to Prometheus's `[a-zA-Z_][a-zA-Z0-9_]*` and values escaped, so
no caller can break out of the exposition; the cardinality of what you
pass stays your decision.

## Health endpoints

The two statuses are what a readiness probe is made of, and they say
different things:

```python
@app.get("/readyz")
def readyz() -> Response:
    status = config.status()

    if config.try_current() is None:
        return Response("no configuration", status_code=503)
    if not status.is_healthy:
        return Response("configuration stale", status_code=503)

    return Response("ok")
```

*Serving something* and *the last attempt worked* are separate
conditions, and a service that conflates them either refuses traffic it
could serve or accepts traffic on a configuration nobody has been able
to reload for an hour. `try_current()` answers the first without raising;
`is_healthy` and `stale_for` answer the second.

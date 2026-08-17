# Telemetry

Two questions an operator asks about configuration, and neither is
answerable from a log line that says "reloaded": *is this process serving
the configuration I deployed*, and *how long has it been serving something
else*. The crate answers them with a record per reload under `tracing`,
and a set of numbers any exporter can publish.

## The crate picks no metrics ecosystem

A library that depends on `prometheus` picks a fight with every
application that chose `metrics`, or OpenTelemetry, or nothing. So this
one depends on none of them, and neither of the two features here pulls a
single crate in:

| Feature | Adds | Depends on |
|---|---|---|
| `tracing` | a span and an event per reload, and the watcher diagnostics | `tracing` |
| `telemetry` | `telemetry::Exposition` — a `ConfigStatus` as Prometheus text | nothing |

Neither raises the MSRV, and with both off there is no code: the module
is not compiled, and the read path is byte-for-byte what it was.

The design is the same one the crate applies to runtimes and to figment.
The library states facts; the binary chooses the machinery. An
application already running a `metrics` recorder or an OpenTelemetry SDK
reads [`status()`](reload-lifecycle.md) in its own exporter and never
touches the `telemetry` module. One that wants a `/metrics` handler and
nothing else uses `Exposition` and pulls in no exporter at all. Spans
reach OpenTelemetry the way every other crate's do — through
`tracing-opentelemetry`, which is the application's dependency.

## What a reload emits

With `tracing`, every install is a `dynamic_config.reload` span, entered
around the reload hooks so that whatever a hook logs is attributed to the
reload that ran it:

```text
dynamic_config.reload
  ├ config     = "my_app::DbConfig"   the configuration type
  ├ reason     = "file-changed"       the ReloadReason, as a category
  ├ generation = 42
  └ outcome    = "installed"
```

and every reload that installs nothing is a `WARN` event:

```text
config     = "my_app::DbConfig"
outcome    = "rejected"
error.kind = "type"
error.path = "pool.max_size"
```

**No value is ever a field.** A key path may be one — that is what makes
a refusal actionable, and it is the same line
[`Error`](validation-diagnostics.md) draws. `reason` is the *category*:
`ReloadReason::FileChanged` owns the path that triggered the reload, and
the field carries `file-changed` without it, because everything
downstream copies that field into a metric label.

**Nothing is emitted on the read path.** `current()` is an atomic load
and stays one; a span per read would be a span per request in every
service using this crate. The records are on reloads, which are rare.

## What a fetch emits

A fetch from a remote store is a `dynamic_config.fetch` span *around* the
round trip — opened before it, so a subscriber has a duration to read —
with an event inside carrying the outcome:

```text
dynamic_config.fetch
  └ outcome = "fetched"

outcome     = "fetched"      an INFO event, inside the span
duration_ms = 12.4
```

and a failure is a `WARN` with the category:

```text
outcome    = "failed"
error.kind = "remote"        or "auth", which will not fix itself by waiting
```

**No field names the store.** The only string a `Remote` has for itself is
its source's `describe()`, which is the store's URL — and a store URL
routinely embeds `user:password@host`. What identifies a fetch in a trace
is the span it is nested in, which is the caller's own; what identifies a
*series* is the label whoever renders the exposition chose. Neither is
derived from the store.

## The metrics

`Exposition` renders one or more `ConfigStatus` values as Prometheus
text. It holds no state between scrapes because it needs none — a status
is a handful of atomic loads and no I/O, so the numbers are recomputed
per scrape and cannot go stale.

```rust,ignore
use dynamic_config::telemetry::Exposition;

async fn metrics() -> String {
    let mut exposition = Exposition::new();

    exposition.add("db", &DbConfig::status());
    exposition.add("http", &HttpConfig::status());

    exposition.render()
}
```

| Name | Type | Labels | Value |
|---|---|---|---|
| `dynamic_config_installs_total` | counter | `config` | installs since the process started |
| `dynamic_config_last_success_seconds` | gauge | `config` | seconds since the serving snapshot landed |
| `dynamic_config_consecutive_failures` | gauge | `config` | failures since the last install; **zero is healthy** |
| `dynamic_config_last_failure_seconds` | gauge | `config` | seconds since the last failure |
| `dynamic_config_last_reload_info` | gauge | `config`, `reason` | `1` |
| `dynamic_config_last_failure_info` | gauge | `config`, `kind` | `1` |

**These names are API.** They end up in dashboards and alert rules, so a
rename is a breaking change and belongs in the changelog under *Changed*.

The `_seconds` and `_failure` families are **absent rather than zero**
where the fact has not happened yet. A configuration that has never been
installed has no staleness, and `0` would read as "installed a moment
ago" — the opposite of the truth.

`last_success_seconds` is the one an alert is written against, because it
is the one nothing else implies:

```yaml
- alert: ConfigurationStale
  expr: dynamic_config_last_success_seconds > 3600
  for: 10m
  annotations:
    summary: "{{ $labels.job }} has not installed configuration in an hour"

- alert: ConfigurationNotReloading
  expr: dynamic_config_consecutive_failures > 0
  for: 5m
  annotations:
    summary: "{{ $labels.job }} is refusing every reload of {{ $labels.config }}"
```

The second one is the reason a bad edit degrading to "no change" is safe
rather than silent: the process keeps serving, and the number says so.

## The remote store's own numbers

`ConfigStatus` answers *did the document install*. It cannot answer *did
the store answer*, and those come apart exactly where it matters: a fetch
that returns a document identical to the one already held is a success
that installs nothing, and a store that has stopped answering leaves a
perfectly healthy `ConfigStatus` behind it.

So a `Remote` records a `RemoteStatus` — the same shape.
The same `FailureStatus`, the same `consecutive_failures` where zero is
healthy, the same recorded-where-it-happens rule, and the same
`Exposition` renders it:

```rust,ignore
exposition.add("db", &DbConfig::status());
exposition.add_remote("db", &DbConfig::remote_sink().status());
```

The sink is the door because the slot is not one: `#[dynamic_config]`
generates the `Remote` as a private accessor, and a `RemoteSink` is the
public handle a watch loop already holds. Taking one only to read the
status costs an atomic load — the generation a sink captures fences
`apply` and nothing else.

| Name | Type | Labels | Value |
|---|---|---|---|
| `dynamic_config_remote_up` | gauge | `config` | `1` if the store answered last time, `0` if not |
| `dynamic_config_remote_fetches_total` | counter | `config` | documents the store has handed over |
| `dynamic_config_remote_last_fetch_seconds` | gauge | `config` | seconds since it last handed one over |
| `dynamic_config_remote_last_fetch_duration_seconds` | gauge | `config` | how long the last pull took |
| `dynamic_config_remote_consecutive_failures` | gauge | `config` | fetches that returned nothing since one did |
| `dynamic_config_remote_last_failure_info` | gauge | `config`, `kind` | `1` |

`remote_up` is **absent before the first fetch**, on the same principle
as the two `_seconds` families: a source that has been installed and
never asked is not *down*, and a `0` at startup is a page nobody should
be woken by.

The name on a series is the caller's own — the same name the
configuration's `add()` was given, so the two halves join in a query.
**Not the store's URL**, and there is no method that takes one: a store
URL routinely embeds `user:password@host`, which makes it a credential
rather than an identifier.

A document that arrives by *push* — a watch loop calling
`remote_sink().apply(..)` — counts as a fetch, because the store did
answer; it reports no duration, because the store crate that made the
round trip is the one that timed it, and the previous pull's number
beside a push's timestamp would describe the wrong fetch.

```yaml
- alert: ConfigurationStoreUnreachable
  expr: dynamic_config_remote_up == 0
  for: 5m
  annotations:
    summary: "{{ $labels.job }} cannot reach the store behind {{ $labels.config }}"
```

## Cardinality

A per-key or per-path label is an unbounded label set, and one
badly-labelled counter is how a Prometheus acquires a million series. So
the bound is stated rather than hoped for.

For `C` configurations and `R` remote sources in an exposition — `R ≤ C`,
because a configuration type has one `Remote`:

| | per scrape | over a process's life |
|---|---|---|
| a `ConfigStatus` | `6 × C` | `(4 + 5 + 10) × C = 19 × C` |
| a `RemoteStatus` | `6 × R` | `(5 + 10) × R = 15 × R` |

so **at most `6 × C + 6 × R ≤ 12 × C` series in a scrape**, and `34 × C`
distinct series over the life of a process. The labelled families draw
from fixed enums — five `ReloadReason`s and ten `ErrorKind`s — and `C` is
the number of configurations a process has, which is a handful.

A per-*store* series multiplies by a bounded number, which is why it
exists. A per-*key* one would not, which is why there is no method that
could make one.

**No key path, file name, store key or configured value can become a
label**, and there is no method that could make one: every sample is
built from a `ConfigStatus`, which holds none of them, and the labels are
the caller's own. That is one notch tighter than the rest of the crate,
which allows a key path in a diagnostic — a metric label is unbounded
*and* a disclosure, and a scrape endpoint is usually the least guarded
surface a process has.

Label values are escaped and label names are coerced to Prometheus's
`[a-zA-Z_][a-zA-Z0-9_]*`, so a section named `x" } 1` cannot forge a
sample line. What a label *costs* — how many distinct values a caller
passes — stays the caller's decision, because only the caller knows.

## In the config server

[The config server](https://dynamic-config-rs.github.io/remote/config-server.html) exposes `GET /metrics`: the same
six families, labelled `application` and `profile` rather than `config`,
for the sections the calling principal may read.

It is **authenticated**, unlike `/healthz` and `/readyz`. Those two are
open because they answer a boolean and disclose nothing — not how many
sections there are, not which one is unhappy — and a metrics endpoint
that could say as little would be no use. One that names sections is an
enumeration of every service the fleet configures, which is exactly what
that server's [threat model](https://dynamic-config-rs.github.io/remote/config-server/threat-model.html) exists to
withhold. So a scraper is a client like any other: give it a token,
grant it the applications it should see, and point Prometheus's
`bearer_token_file` at it.

## What is absent

**A reload counter broken down by reason and outcome.** It would need
state this crate does not keep — `status()` records the *last* reason and
the streak, not a tally per category — and inventing a second set of
counters beside it would give two surfaces that disagree after the first
bug. `installs_total` is the count that already exists, because a
generation *is* the number of installs.

**A duration for the load.** The reload span covers the install and the
hooks, not the read of the sources, and how long a file took to parse is
not what anybody alerts on. Staleness is, and that is a gauge. A *fetch*
is timed, because a network round trip is the one step whose duration is
a fact about somebody else's service.

**A histogram of fetch durations.** `last_fetch_duration_seconds` is a
gauge of one measurement and is named for it, so that no dashboard
reaches for `histogram_quantile` against buckets that are not there.
Percentiles would mean a reservoir per source — state whose shape a
library has no business choosing — and an application that wants them
times its own `refresh_remote` call in its own recorder.

**A store label.** The only name a source has for itself is its URL, and
a store URL routinely embeds `user:password@host`. Series are named by
the caller instead, which is bounded as well as safe.

**An exporter.** No `metrics`-crate recorder, no OpenTelemetry SDK, no
HTTP handler. Each would be a dependency chosen on the application's
behalf, and the numbers are available to every one of them through
`status()`.

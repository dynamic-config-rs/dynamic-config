# The Reload Lifecycle

Reloading the configuration does not reload anything *built from* the
configuration. A changed `pool_size` swaps a number in a snapshot; the pool
sized by the old number is still there, still that size. This chapter is
about the boundary — what the crate does on a reload, what it deliberately
leaves to you, and the surface for doing your half.

```text
file edited
    ↓
load → validate → snapshot → atomic swap      ← the crate's half, all of it
    ↓
pools, clients, listeners built from config   ← your half, on purpose
```

The crate stops at the swap because it cannot do better than you: it does
not know whether a changed `database_url` means "drain and reconnect", "keep
serving and reconnect lazily", or "that field was never supposed to change —
log and ignore". Whatever it guessed would be wrong for somebody, silently.

## The surface for your half

**`on_reload(old, new)`** — a callback on every later reload, for the life
of the process. It runs on whichever thread performed the reload — the
watcher thread, usually — so keep it short and move real work elsewhere:

```rust
DbConfig::on_reload(|old, new| {
    if old.pool_size != new.pool_size {
        pool.resize(new.pool_size);          // cheap: do it here
    }
    if old.database_url != new.database_url {
        reconnect_tx.send(()).ok();          // expensive: signal, don't do
    }
});
```

A callback that panics is caught and logged; the callbacks after it still
run. Installing the first snapshot is not a reload, so `init()` fires
nothing.

**`on_reload_scoped(..) -> HookGuard`** — the same, until the guard drops.
For subsystems with a shorter life than the process.

**`on_reload_with(event)`** — the same list, told *why*. See
[Why a reload happened](#why-a-reload-happened) below; it is the form to
reach for when the reaction depends on what moved the configuration rather
than only on what the configuration now says.

**`changes()`** — the awaitable form, with the `async` feature. A task that
would rather await than be called back:

```rust
let mut reloads = DbConfig::changes();

loop {
    let config = reloads.changed().await;
    pool.resize(config.pool_size).await;     // async work is fine here
}
```

This is also why there is no `on_reload_async`: `changes()` *is* the async
reload event, on your executor instead of the watcher thread, free to await
whatever the reaction needs. A handle created before `init()` sees the
initial install as its first change, so it doubles as "wake me when
configuration exists".

**`ReloadGroup`** — when several configuration types must move together or
not at all. Every member loads and validates before any member installs, so
a failure leaves all of them on their previous snapshots.

One honest limit: the group's promise is all-or-nothing *installation*,
and the commits are still separate swaps — a reader can observe member
A's new generation next to member B's old one for an instant. For the few
cases where even that instant matters — a certificate and the port it is
served on — the answer is structural, not more machinery: **one type
holding both concerns, one section, one swap, one generation.**

```rust
#[derive(Debug, Deserialize)]
struct Tls {
    certificate_path: String,
    port: u16,          // moves with the certificate, atomically, always
}
```

A reader of `Tls` can never see a torn pair, because there is no pair —
there is one value, replaced whole, which is the crate's founding
guarantee. This works identically on the attribute surface, on
[`Dynamic`](dynamic-instances.md), and inside a `ReloadGroup` whose other
members tolerate the instant. Reach for the group when types are owned by
different modules and "roughly together" is enough; merge the sections
when it is not. A `Bundle` helper was considered and refused: it would be
a rename of "define one struct", and machinery that restates a design
decision teaches people to skip the decision.

**`set_blocking_executor`** — where the blocking half of an async reload
runs. By default it is a thread per load; the `tokio` feature routes it to
the blocking pool; anything else can be installed by hand. Reloads are rare
by design, so the default is fine until measured otherwise.

## Why a reload happened

`on_reload(old, new)` hands you two snapshots and nothing else, which means
a hook cannot tell an edited file from a manual `reload()` from a document a
remote store pushed. By the time the callback runs they are the same swap.
The reason has to be recorded where the reload was *triggered*, so that is
where it is recorded, and `on_reload_with` is the form that receives it:

```rust
use dynamic_config::ReloadReason;

DbConfig::on_reload_with(|event| {
    match &event.reason {
        // The path is what opened the debounce window — the trigger, not
        // the diff. `changed_paths(old, new)` is the diff.
        ReloadReason::FileChanged(path) => {
            tracing::info!(file = %path.display(), generation = event.meta.generation, "reloaded");
        }
        // Nothing was serving before this; `event.previous` is `None`.
        ReloadReason::Initial => tracing::info!("configuration is live"),
        // The sources would not load and the cache stood in.
        ReloadReason::Recovered => alert("running on the last known good configuration"),
        _ => {}
    }
});
```

| Reason | Produced by |
|---|---|
| `Initial` | `builder.init()` — the call that establishes a configuration. |
| `FileChanged(path)` | The file watcher. The path is the one whose event opened the debounce window. |
| `RemoteChanged` | `RemoteSink::apply(..)` — a store's watch loop pushed a document. |
| `Manual` | The program: `reload()`, `replace(..)`, `ConfigCell::store`, a `ReloadGroup` commit. |
| `Recovered` | `init()` fell back to the last-known-good cache. |

`ReloadReason` is `#[non_exhaustive]`, so a `_` arm is required and a new
variant will not break the match. `reason.as_str()` is the category without
the path — the shape a metric dimension wants, since a file path is
unbounded cardinality and `"file-changed"` is not.

Two differences from the pair form are worth stating plainly:

- **The first install fires an event.** `previous` is `None`, because there
  was no configuration before it. `on_reload` cannot say that — its
  signature has nowhere to put it — so it stays silent for `init()`, and
  that has not changed.
- **`{:?}` on an event prints no configuration.** The reason, the
  generation, and whether there was a previous snapshot; never `T`. An event
  is a diagnostic, a `{:?}` of one lands in a log, and a configuration holds
  passwords. Reach for `event.current` when you want the values.

Everything else is identical: one list, one registration order, the same
panic isolation, and the same absence of a defined order across overlapping
reloads.

A program that detects its own changes — a store this crate has no adapter
for, a control plane pushing over a socket — can label its own reloads with
`builder.reload_with(reason)`.

## Operating a configuration

`status()` answers the questions that arrive at three in the morning, in one
struct and without touching a source:

```rust
let status = DbConfig::status();

status.generation;            // which generation is live
status.stale_for();           // how long ago it landed
status.last_reason;           // why it landed
status.consecutive_failures;  // failures since one worked — zero is healthy
status.last_failure;          // when the last one was, its kind and key path
```

It is a handful of atomic loads. Nothing is re-read, nothing is recomputed,
nothing can block — so an exporter may call it per scrape:

```rust
// the application's own HTTP surface, where the authentication already is
router.route("/internal/config", get(|| async {
    let status = DbConfig::status();
    format!("generation {} healthy {}", status.generation, status.is_healthy())
}));
```

Three things it deliberately is not.

**It is not a value leak.** A status carries key paths, counts, timestamps,
generations and error kinds — never a configured value. `last_failure` keeps
the failure's `ErrorKind` and the key path it was reported at, and not its
message: an error's `Display` is value-free by policy, but a struct that
*stores* free text is one careless construction away from carrying a value
into every log that prints a status. This is the crate's rule everywhere,
and a status struct is exactly the thing somebody `{:?}`s into a log.

**It is not a second source of truth.** Every field is recorded where it
happens. There is no `last_success` because an install *is* the success —
`loaded_at` is when the last one was — and no source list, because "which
sources would be read" is a question about the *next* load, which
[`check()`](validation-diagnostics.md) answers against the sources rather
than from a cache of them that could go stale.

**It is not a CLI subcommand.** `dynamic-config status` would need a channel
into a running process — a socket, a pid file — which is a different product
with its own threat model. The exporter above is the honest answer, and it
belongs to the application. What the CLI can answer is *what would load now,
here*, and that is [`dynamic-config check`](cli.md).

Like `meta()`, a status is assembled from several loads, so a reload landing
mid-call can leave one field an install ahead of another. It is for
operators, not for correctness.

## Watching the watcher

With the `tracing` feature, every watcher reload is a `config_reload` span
(target `dynamic_config`) whose events carry the outcome and duration —
enough to alert on "has not reloaded cleanly in an hour" without parsing
message strings. Without the feature, the stderr lines carry the duration.

For the audit half — *what* moved, paths only, never values:

```rust
DbConfig::on_reload(|old, new| {
    for change in dynamic_config::changed_paths(old, new).unwrap_or_default() {
        tracing::info!(target: "audit", %change, "configuration changed");
    }
});
```

## Patterns that hold up

- **Diff before acting.** `on_reload` hands you both snapshots; compare the
  fields you care about and do nothing when they did not move. Most reloads
  change something else.
- **Signal, don't do.** For anything expensive — reconnects, migrations,
  cache flushes — send a message to the subsystem that owns the resource
  and let it act on its own thread, at its own pace.
- **Fields that must not change should say so.** The builder's
  `validate(f)` hook can compare against the running value via
  `try_current()` and refuse the reload — the old snapshot stays, the
  error is reported, and nobody reconnects to a database because of a typo.
- **Readers never participate.** A request handler calls `current()` once
  and uses that snapshot to the end; it does not need to know reloads exist.
  The lifecycle above is for the handful of places that own long-lived
  resources.

## When a reload fails, and whether to keep trying

A watch loop that treats every failure alike either gives up on something
temporary or hammers something permanent. The error's kind is the
distinction:

| Kind | What it means for the loop |
|---|---|
| `Remote` | the store is unreachable, or answered badly — **back off and retry**; this often fixes itself |
| `Auth` | a credential was rejected or could not be obtained — **stop and report**; waiting will not fix it |
| `Io`, `Parse`, `Invalid` | the source is there and wrong — the previous snapshot keeps serving, and somebody has to edit something |

`ErrorKind::Auth` exists for exactly this decision. Before it, "the token
expired" and "the network is down" arrived as the same kind, and a loop
could only guess.

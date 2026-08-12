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

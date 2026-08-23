# Hot Reload & Watching

## Reading is lock-free

The snapshot lives in a `OnceLock<ArcSwap<T>>`. `current()` clones an `Arc` out
of it, so a reload never blocks a request handler, and a reader that already
holds an `Arc` keeps its own generation.

**Call `current()` once per unit of work** and reuse the `Arc`. Calling it twice
inside one request can straddle a reload and observe two configurations.

## Reloading cannot take the process down

A reload re-runs the builder's `load()`. If the new configuration is invalid,
or a file is caught half-written, the error is reported and the previous
snapshot stays in place. A bad edit degrades to "no change".

## `watch`

```rust
let builder = DbConfig::builder("db").file("config.toml");
builder.init()?;

builder.watch(Duration::from_millis(250))?.detach();
```

`watch(debounce)` on the builder reloads the snapshot when a file changes.
Requires the `watch` feature. Each reload loads through this builder and
installs into the type's snapshot, firing `on_reload` hooks and waking
`changes()` exactly as any other install does; a configured
[cache](persistence.md#cache) is rewritten after each clean reload.

**The returned handle owns the watcher** — dropping it stops watching:

```rust
builder.watch(debounce)?.detach();       // a server: watch for the whole process
let _watch = builder.watch(debounce)?;   // a test, a subcommand: stop with the scope
```

One watcher per type: a second `watch()` while one runs is `AlreadyExists`,
whoever started the first.

The watcher observes the **directory** holding each file rather than the file
itself: editors and `mv`-based atomic saves replace the inode, which silently
detaches a file-level watch. That is also what makes a Kubernetes ConfigMap
update — delivered as a `..data` symlink swap — visible at all.

A reload that fails is logged and the previous snapshot is kept.

## `debounce`, and the two numbers beside it

The `Duration` handed to `watch`. One editor save typically emits several
filesystem events; waiting out a quiet period collapses them into one reload.
250 milliseconds is a reasonable place to start.

Two more numbers govern the same wait, and both used to be unreachable —
one written into the loop, the other a process-wide setter. `WatchOptions`
names all three, and a bare `Duration` still converts, so the one-argument
form is unchanged:

```rust
use dynamic_config::WatchOptions;

let handle = Config::builder("svc").watch(
    WatchOptions::new(Duration::from_millis(100))
        // How long the quiet period may keep restarting before the reload
        // happens regardless. Four debounces unless it is set: without a
        // bound, a sustained storm of writes starves the reload forever.
        .ceiling(Duration::from_secs(2))
        // The pause between the window closing and the files being read.
        // An atomic save renames a temporary file into place, and the
        // rename can be observed a hair before the new inode is visible.
        .atomic_save_grace(Duration::from_millis(50)),
)?;
```

The grace still defaults to the process-wide `set_atomic_save_grace`, and
that is the right default: it compensates for the *filesystem*, which every
watcher in the process shares. Naming it per watch is for the program
watching a local disk and a network mount at once.

## Ending a watch when the process does

`stop()` and `detach()` are the two ends of the range: stop now, or run
forever. `run_until` is the one a server actually wants —

```rust
Config::builder("svc")
    .watch(Duration::from_millis(100))?
    .run_until(shutdown_signal())
    .await;
```

— because a watch is not something to remember to stop, it is something
that ends when the process is winding down. No runtime is imposed: the
future is driven by whichever executor is already running the caller, and
this adds no second cancellation mechanism — it ends the watch exactly the
way dropping the handle does. `RemoteWatch` has the same method for the
same reason.

## Polling instead of notification

```rust
builder.watch_with(
    Duration::from_millis(250),
    WatchMode::Poll { interval: Duration::from_secs(2) },
)?;
```

`watch_with` chooses the detection strategy explicitly: `WatchMode::Native`
is what plain `watch` uses, and `WatchMode::Poll` re-reads on an interval
instead of waiting for a notification.

Polling is needed because inotify and its equivalents do not fire on many
network and overlay filesystems — NFS, some Docker bind mounts, some CI
runners. The failure is **silent**: the watch registers and simply never
delivers, so there is nothing to detect and fall back from. It has to be
chosen explicitly.

Each tick compares **contents**, not only timestamps, and that is a
correctness decision rather than a thorough one: a filesystem timestamp is
compared here in whole seconds, so an edit landing in the same second as
the previous scan would otherwise be invisible — and stay invisible, since
the next scan compares against the value it just recorded. A deployment
that writes a file moments after the watcher starts is exactly that case.

What it costs is a read where there would have been a `stat`, once per
interval per file in the directories being watched. Configuration files are
small and few, and a watcher that misses edits is the failure polling was
chosen to escape. Choose the interval accordingly: two seconds over a
network mount is a different bill from fifty milliseconds.

## Watching a remote store

A file watcher is one half. The other is a store: a change written to
Consul, etcd, Vault or a config server should reach the process the same
way an edited file does.

```rust
let sink = DbConfig::remote_sink();
let handle = RemoteWatch::new();
let watching = handle.watching();

std::thread::spawn(move || {
    store.watch(&watching, Duration::from_secs(30), |document| {
        sink.apply(document)          // installs, reloads, wakes the readers
    })
});
```

Blocking, so it belongs on a thread of its own; the async twin is
`watch_async` and cancelling it is dropping the future. Dropping the
`RemoteWatch` stops the loop — `detach()` says *this one really should run
forever*.

**The sink is what makes a delivery a reload.** `RemoteSink::apply` is the
piece that installs the document *and* runs the configuration's reload —
validation, hooks, `changes()`, the last-known-good cache. Its twin
`sink.failed(&error)` records an attempt that came back with nothing, so a
watch whose store has been down for an hour stops reporting it as
reachable.

`Remote::watch` is the other half, and it does less on purpose: it keeps
the *document* fresh in the remote slot, the way `refresh()` does, and
reloads nothing. A `Remote` is a store handle, not a configuration — it
has no type to deserialize into and no hooks to fire. Reach for it when
something else decides when to reload; reach for the sink when a change
in the store should reach `current()` on its own.

**What the interval means depends on the store**, and the store says which:

| capability | what a watch does | what the interval means |
|---|---|---|
| `Native` | the store's own mechanism — a blocking query, a stream, a subscription | a resync, because a stalled stream looks exactly like a store where nothing changed |
| `Conditional` | asks whether it changed, and reads the document only when it did | how often to ask |
| `Interval` | re-reads the whole document | how often to read |

```rust
match REMOTE.watch_capability() {
    Some(WatchCapability::Native) => // changes arrive as they happen
    Some(WatchCapability::Conditional) => // a cheap question on a timer
    Some(WatchCapability::Interval) => // the whole document on a timer
    None => // no store is installed
}
```

A store with no watch of its own is still watched: the default polls, and
**only a document that differs from the last one is installed**, so a
process does not fire every reload hook it has once an interval.

The waits are not flat, and they are not jittered the same way twice —
because the two waits are solving different problems.

A **healthy** wait is spread by up to a quarter in either direction. Fifty
replicas started by one rollout would otherwise hit the store
simultaneously, every interval, forever. The interval is a promise about
how often a store is read, and a band is enough to break the lockstep
without changing what was promised.

A wait **after a failure** is drawn from the whole range instead — anywhere
between nothing and the full backoff, which doubles up to a ceiling. That
is the shape that actually decorrelates a fleet, and a recovering store is
the one moment it matters: a thousand agents that failed at the same
instant have been counting the same doubling ever since, and a band a
quarter wide would land them back on the store in a clump.

The two are not interchangeable. Drawing a *healthy* wait from the whole
range would halve its mean, which is not a jitter policy — it is a
different interval, and it doubles the read rate of every store in the
fleet.

`Pace` is that policy on its own, for a store crate writing its own loop:

```rust
let mut pace = Pace::new(Duration::from_secs(30));

while watching.keep_going() {
    match fetch() {
        Ok(document) => pace.succeeded(),
        Err(error) => pace.failed(),
    }

    pace.wait(&watching);
}
```

A failing fetch does **not** end the watch. Outliving an outage is what a
watch is for, so a failure is waited out and tried again.

**Recording it is the caller's**, and worth wiring: the default watch backs
off and says nothing, because a `RemoteSource` is handed a store and a
callback and has no way to reach the status a `Remote` keeps. A loop built
on the sink calls `sink.failed(&error)` — the store crates' `reporting_to`
does it for you — and without that, `status().reachable()` goes on
answering `true` for as long as the outage lasts.

## What a store may say about a document

A `Fetched` is the bytes and the format, and two things a store may know
about them.

**A revision**, where the store has one. `Revision::Counter` for a number
that grows — a Vault KV version, a Consul index, an etcd revision —
and `Revision::Opaque` for one that only ever equals or differs, like an
ETag or an object hash:

```rust
Fetched::new(text, Format::Json).with_revision(Revision::Counter(version))
```

The two are compared differently, and the difference is in the type rather
than in a convention: `supersedes` orders a `Counter` and can only report
*changed* about an `Opaque`, because an ETag carries no order.

This is what makes the install fence about *recency* rather than identity.
Two fetches can be in flight at once — a watch delivery and a resync, say —
and the one that finishes second is not necessarily the one that read
later. A `Counter` that is not greater than the installed revision is now
refused, and an identical `Opaque` installs nothing.

**A lease**, where the store mints the document rather than storing it:

```rust
pub trait RenewableSource: RemoteSource {
    fn renew(&self, lease: &Lease) -> Result<Lease, Error>;
    fn revoke(&self, lease: &Lease) -> Result<(), Error>;
}
```

`Lease { id, ttl, renewable }` rides on the `Fetched`, and the trait is how
a caller extends or hands back what the store issued. Blocking rather than
async, because the stores that issue leases are blocking clients whose
callers already run them off the reactor. `Lease` redacts its own `Debug`:
a lease id is capability-shaped and sits beside generated credentials.

Both fields are additive — `Fetched::new` still means "no revision, no
lease" — and `Fetched` is `#[non_exhaustive]`, so the next thing a store
learns to report will not be a breaking change.

## Three things a slot does on the caller's behalf

- **A refresh is single-flight.** N callers arriving at once make one round
  trip; the rest wake, find the newer revision already installed, and
  return. Watch, resync and an explicit `refresh()` all reach the same
  document, so they no longer race each other to fetch it.
- **A document has a ceiling.** `MAX_DOCUMENT_BYTES` is 8 MiB, enforced
  where the bytes are read so the allocation never happens, and settable
  per slot with `set_max_document_bytes`. The refusal names the limit and
  the store, never the content.
- **"Not there" is its own answer.** A document that has been deleted
  fails with `ErrorKind::Absent` rather than the category a watch loop
  waits out. Waiting cures an outage; it does not bring back a key, and a
  caller that cannot tell the two apart serves a deleted secret forever
  with every health check reporting fine.

## Reacting to a reload

```rust
DbConfig::on_reload(|previous, current| {
    if previous.pool_size != current.pool_size {
        pool.resize(current.pool_size);
    }
});
```

The callback runs on whichever thread performed the reload — the watcher
thread, usually — so keep it short. Installing the first snapshot is not a
reload, so `init()` does not fire it. A callback that panics is caught and
logged; the callbacks after it still run. With the `async` feature, `changes()`
is the same idea for a task that would rather await than be called back.

`on_reload` is permanent — right for wiring that lives as long as the process.
A subsystem with a shorter life uses `on_reload_scoped`, which returns a
`HookGuard`; the callback runs until the guard is dropped:

```rust
let guard = DbConfig::on_reload_scoped(|_previous, current| {
    metrics.set_pool_size(current.pool_size);
});

drop(guard); // the callback is unregistered
```

The guard is `#[must_use]`: binding it to `_` drops it immediately, and the
callback never fires.

Reloading the configuration reloads nothing *built from* it — a changed
`database_url` does not reconnect anything. That boundary, and the patterns
for your side of it, get their own chapter:
[The Reload Lifecycle](reload-lifecycle.md).

## All of it, or none of it

Two structs over one file reload independently, and for a moment after an edit
one is new while the other is old. Usually nobody notices. When it matters — a
certificate path and the port it is served on — group them:

```rust
let group = ReloadGroup::new()
    .with::<ServerConfig>()
    .with::<TlsConfig>();

group.reload()?;
```

Every member loads and validates before any member is installed, so a failure
anywhere leaves *every* member on its previous snapshot — including the ones
that loaded cleanly. The commits are not one atomic operation; they are three
`Arc` swaps with no fallible work between them, which is the part that actually
goes wrong.

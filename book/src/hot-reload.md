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

## `debounce`

The `Duration` handed to `watch`. One editor save typically emits several
filesystem events; waiting out a quiet period collapses them into one reload.
250 milliseconds is a reasonable place to start.

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
chosen deliberately.

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

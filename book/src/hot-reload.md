# Hot Reload & Watching

## Reading is lock-free

The snapshot lives in a `OnceLock<ArcSwap<T>>`. `current()` clones an `Arc` out
of it, so a reload never blocks a request handler, and a reader that already
holds an `Arc` keeps its own generation.

**Call `current()` once per unit of work** and reuse the `Arc`. Calling it twice
inside one request can straddle a reload and observe two configurations.

## Reloading cannot take the process down

A reload re-runs `load()`. If the new configuration is invalid, or a file is
caught half-written, the error is reported and the previous snapshot stays in
place. A bad edit degrades to "no change".

**`start_watch()` returns a handle, and dropping it stops the watcher.** A
server calls `.detach()` to watch for the rest of the process; anything with a
lifecycle — a test, a library, a subcommand — binds the handle so watching stops
when the thing being configured goes away.

```rust
Config::start_watch()?.detach();       // a server
let _watch = Config::start_watch()?;   // a test, a subcommand
```

The watcher observes the **directory** holding each file rather than the file
itself: editors and `mv`-based atomic saves replace the inode, which silently
detaches a file-level watch. That is also what makes a Kubernetes ConfigMap
update — delivered as a `..data` symlink swap — visible at all.

## `watch`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch)]
```

Generates `start_watch()`, which reloads the snapshot when a file changes.
Requires the `watch` feature.

**The returned handle owns the watcher** — dropping it stops watching:

```rust
Config::start_watch()?.detach();       // a server: watch for the whole process
let _watch = Config::start_watch()?;   // a test, a subcommand: stop with the scope
```

Directories are watched rather than files, because editors and `mv`-based atomic
saves replace the inode. Kubernetes ConfigMap updates arrive as a `..data`
symlink swap and are recognised as changes.

A reload that fails is logged and the previous snapshot is kept.

## `debounce`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, debounce = 500)]
```

Quiet period in milliseconds before a reload fires. One editor save typically
emits several filesystem events; waiting collapses them into one reload. Must be
non-zero. Requires `watch`.

## `poll` / `poll_interval`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, poll_interval = 2000)]
```

Detect changes by re-reading on an interval instead of by notification. `poll`
alone uses 2000 ms.

Needed because inotify and its equivalents do not fire on many network and
overlay filesystems — NFS, some Docker bind mounts, some CI runners. The failure
is **silent**: the watch registers and simply never delivers, so there is nothing
to detect and fall back from. It has to be chosen deliberately. Requires `watch`.

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

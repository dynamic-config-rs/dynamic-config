# Async & asyncio

Nothing blocking ever runs on your event loop, and nothing on the loop is
required to use this library. Both halves matter: a service that reads
configuration should not stall its loop on disk I/O, and a script that
has no loop should not have to start one.

## The shape

```python
await config.init_async()          # load, validate, install
candidate = await config.load_async()
await config.reload_async()

model = await config.changed_async(timeout=30)   # the next install, once

async for db in config.changes():                # every install, forever
    await pool.resize(db.pool_size)
```

Every `_async` method is the synchronous one performed on a worker
thread, with the GIL released for the blocking part — reading and parsing
files. What comes back onto the loop is the finished model. The
synchronous methods are not deprecated shadows of these; they are the
right call from a thread, a script or a test.

## Why a worker thread rather than "native async"

The engine's work is filesystem I/O and CPU: reading files, merging
layers, deserializing. There is no socket to await and nothing to
overlap, so an `async` implementation would still block a thread — it
would just be less honest about which one. Handing the work to an
executor and awaiting the result is what "async file I/O" means in
CPython anyway; `asyncio.to_thread` is the same mechanism.

This is the same decision the Rust crate makes. There, `load_async`
sends the load to a blocking worker — a fresh thread by default, or
tokio's blocking pool with the `tokio` feature — precisely so that no
executor thread is parked on a `read()`. The Python binding inherits the
policy rather than reinventing it.

## Which pool pays for the blocking half

By default the work goes to the event loop's own executor — the one
`run_in_executor(None, ...)` uses, shared with everything else in the
process that calls it. A service that would rather not queue behind an
unrelated batch job gives configuration its own:

```python
from concurrent.futures import ThreadPoolExecutor
import dynamic_config

dynamic_config.set_executor(ThreadPoolExecutor(2, thread_name_prefix="config"))

# or for one configuration only
config = DynamicConfig(Database, key="db", executor=pool)
```

This is the Python-side twin of the Rust crate's
`set_blocking_executor`, and answers the same question. What it is *not*
is tokio: the Rust `tokio` feature exists so that a Rust program's async
loads land in tokio's blocking pool rather than on a fresh thread. Here
the awaiting side is Python's loop, which cannot await a tokio task, so
the wheel does not carry tokio — it would be a runtime nobody awaits, in
every wheel, for every user. The executor above is the knob that
actually changes where the work runs.

**Waits stay on the default executor**, whatever you configure. A wait is
a parking spot rather than work, and parking several of them in a pool
you sized for work is how that pool starves — three `changes()`
iterators against a two-worker executor would otherwise deadlock the
reload they are waiting for.

## Waiting for a reload

Two shapes, because two things want to wait:

**`changed_async(timeout=…)`** — one await, one answer. For a task that
needs the *next* configuration and then moves on. Returns `None` when the
timeout elapses first.

**`changes()`** — an async iterator over every install from here on. For
the long-lived task that follows configuration for the life of the
service:

```python
async def follow(config, pool):
    async for db in config.changes():
        if db.pool_size != pool.size:
            await pool.resize(db.pool_size)
```

Both wait in bounded slices with the GIL released, so **cancelling
either is noticed within a quarter second** rather than at the next
reload — which may never come. Cancel the task and the engine is
untouched; a reload afterwards behaves exactly as it would have.

Neither is tied to asyncio's implementation details, so uvloop drives
them, and so does anything else that provides a running loop and an
executor.

## Loading several configurations at once

`DynamicConfig` is a value, so a service with a database file, a cache
file and a feature-flag file has three of them — and three loads that do
not need to queue:

```python
await asyncio.gather(
    database.init_async(), cache.init_async(), features.init_async()
)
```

Each keeps its own watcher, its own generation and its own followers, so
a flag flipping does not re-parse the database file or wake anything
watching it.
[`examples/13_asyncio_many_files.py`](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/13_asyncio_many_files.py)
is the whole shape, executor included.

### The same thing, on the model classes

The other shape is the decorator: the configuration lives *on the model
class*, so any module that can import `Database` can ask
`Database.current()` without being handed a configuration object first.
The async surface is reached through `Model.config`, and everything
above applies unchanged:

```python
@dynamic_config(key="db", files=["database.toml"], env="APP_")
class Database(BaseModel):
    host: str = "localhost"
    pool_size: int = 8

@dynamic_config(key="flags", files=["flags.toml"], env="APP_")
class Flags(BaseModel):
    new_checkout: bool = False

# Three files, one await; the loop is free while they are read.
await asyncio.gather(
    Database.config.init_async(), Cache.config.init_async(), Flags.config.init_async()
)

watch = await Flags.config.watch_async(debounce=0.25)

async for flags in Flags.config.changes():
    ...                                  # one follower per configuration
```

`Model.current()` stays synchronous everywhere — it is an attribute
lookup on a cached instance, so there is nothing to await, on the loop or
off it. The decorator does not load at import time (`init=False` is the
default), which is what makes decorating at module level safe: importing
a module should not begin filesystem work, and a loop that does not exist
yet cannot be the thing loading.

[`examples/14_async_decorator_services.py`](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/14_async_decorator_services.py)
runs three decorated services on one loop — concurrent loads, a watcher
and a follower each, and generations that prove one team's edit left the
other two configurations alone.

## Reading inside a request

```python
@app.get("/health")
async def health():
    db = config.current()      # once, at the top
    await do_work(db.host)
    return {"host": db.host}   # the same value, whatever landed meanwhile
```

The same line works in a synchronous endpoint — FastAPI runs those on a
worker thread — because the read is thread-safe and needs no loop: the
model is immutable and the swap is atomic, so a reload landing
mid-handler cannot tear the value in hand. See
[Web Frameworks](frameworks.md) for both styles side by side.

`current()` is an attribute lookup — no `await`, no boundary crossing, no
lock a writer can hold. Read it once per request and use that value for
the whole request: a reload landing halfway through would otherwise show
one request two configurations, which is the one bug hot reload
introduces if you let it.

## Watching, from a loop

`config.watch(...)` starts a background thread, not a task, because the
filesystem notification backend is a thread-shaped thing on every
platform. It needs no loop and does not interact with yours: when a
reload lands, it validates on the watcher thread, publishes, and wakes
whatever is awaiting `changes()` on your loop.

```python
watch = await config.watch_async(debounce=0.25)
watch.detach()        # for the life of the process
```

`watch_async`, and not because the watcher needs it — it is a thread
either way. What the await moves off the loop is *starting* it:
resolving the directories to observe, registering each with the
notification backend, spawning the carrier thread. Natively that is a
fraction of a millisecond, growing with the number of directories.
`poll_interval` is the case that earns the twin: the poll backend takes
a baseline scan of everything it watches before it can report a change,
which measures single-digit milliseconds over a large directory and
worse over the network filesystems that are the reason to poll at all. A
startup handler runs once and would survive the sync call; a loop that
is answering requests should not be the thing waiting on `readdir`.

Stopping needs no twin. `Watch.stop()` drops the backend, which closes
the channel the watcher thread is parked on, and returns — it does not
join the thread and does not wait out a debounce window, so a reload
already in flight finishes on its own thread while `stop()` has long
returned. Call it directly from a shutdown handler.

Hooks registered with `on_reload` also run on the watcher thread. If a
hook needs to touch loop-owned state, hand the work over rather than
doing it there:

```python
loop = asyncio.get_running_loop()
config.on_reload(lambda old, new: loop.call_soon_threadsafe(queue.put_nowait, new))
```

That is the same advice the Rust
[reload lifecycle](../reload-lifecycle.md) gives: compare, then signal
the thing that owns the resource.

# Callbacks

Loading configuration is the easy half. The half that decides whether hot
reload is *useful* is what happens next: a pool that has to be resized, a
client that has to be rebuilt, an audit line somebody will read at three
in the morning.

```python
@config.on_change("pool.max_size")
def resize(old, new):
    pool.resize(new.pool.max_size)
```

That is the whole idea. The rest of this page is what the arguments mean,
what a hook may and may not do, and the four other shapes the same thing
takes.

## The five shapes

| Shape | Runs |
|---|---|
| `config.on_reload(hook)` | after every install |
| `@config.on_reload` | the same, with the function's name kept |
| `@config.on_change("path", …)` | only when one of those paths moved |
| `with config.on_reload(hook):` | for the length of the block |
| `async for model in config.changes()` | on your event loop, no callback |

All but the last hand back a [`HookGuard`](reference.md#hookguard), which
unregisters on `close()` or at the end of a `with`.

## What the arguments mean

```python
def hook(old: Model | None, new: Model) -> None:
```

`old` is `None` for the first install and the previous model after that.
That is how a hook tells *starting up* from *something changed* without
keeping a flag:

```python
@config.on_reload
def audit(old, new):
    if old is None:
        log.info("loaded %s:%s", new.host, new.port)
    else:
        log.info("reloaded: %s", ", ".join(str(c) for c in changed_paths(old, new)))
```

[`changed_paths`](reference.md#changed_pathsprevious-current) is the
audit half of a reload: which paths moved, never what they moved to. It
compares secrets — comparing the mask would make two different passwords
look equal — and reports paths only, so the line above is safe to log.

A read *inside* a hook sees the new model: this configuration's own
publish hook is registered first, deliberately, so `config.current()`
agrees with the `new` argument rather than lagging it by one install.
`config.generation` is already bumped, too.

## The decorator keeps the function

`on_reload` returns the guard, and the guard forwards calls to the hook,
so decorating does not take your function away:

```python
@config.on_reload
def resize(old, new):
    pool.resize(new.pool.max_size)

resize(None, config.current())   # still the function — useful in a test
resize.hook                      # the undecorated one, if you need it
resize.close()                   # and still the registration
```

Without that, `@config.on_reload` would quietly rebind the name to
something you cannot call, which is the kind of surprise a decorator
should never be.

## Filtering: react to a path, not to an install

A reload installs a whole model whether or not the field you care about
is in it — a neighbouring key changed, an operator re-saved the file, a
watcher fired on a touch. Rebuilding a connection pool on every one of
those is churn a service can feel:

```python
@config.on_change("host", "port")
def reconnect(old, new):
    pool.rebuild()          # expensive; only when the address really moved
```

Details worth knowing:

- **Paths are dotted**, as everywhere else in this crate, and a path
  naming a table covers what is inside it: `on_change("pool")` fires for
  `pool.max_size`.
- **The first install always counts as a change**, because there is
  nothing to compare it against. A hook that *sets something up* runs at
  startup rather than waiting for the first edit — register it before
  `init()` and it will.
- **The comparison is `changed_paths`**, so a secret that changed is
  noticed without being printed.
- It is a decorator factory: `config.on_change("port")(hook)` is the same
  thing written without the `@`.

## What a hook may do, and what it should not

A hook runs **on the thread that reloaded** — the watcher's thread, or
the caller's for an explicit `reload()`. So:

- **Do** compare, log, set a flag, put something on a queue, call
  `loop.call_soon_threadsafe`.
- **Do not** rebuild a connection pool, make a network call, or take a
  lock a request handler holds. A slow hook delays the *next* reload and
  holds a thread the watcher needs.

The rule is the one the Rust
[reload lifecycle](../reload-lifecycle.md) gives: compare, then signal
the thing that owns the resource.

```python
work: queue.Queue[Service] = queue.Queue()
config.on_reload(lambda old, new: work.put(new))   # the hook ends here
```

On an event loop, the same handover is `call_soon_threadsafe`:

```python
loop = asyncio.get_running_loop()
config.on_reload(lambda old, new: loop.call_soon_threadsafe(queue.put_nowait, new))
```

…though if you are on a loop already, [`changes()`](async.md) is usually
the better answer: same events, awaited rather than pushed, and the body
runs on the loop where it can `await`.

## When a hook raises

The raise is *reported*, through Python's unraisable channel
(`sys.unraisablehook`), and the hooks after it still run. The install
itself already happened — a hook is a reaction, not a veto. What vetoes
a bad configuration is validation, which runs
[before anything installs](../python.md#where-validation-happens-and-why-it-matters).

If you want a hook's failure to be loud, make it loud yourself:

```python
@config.on_reload
def resize(old, new):
    try:
        pool.resize(new.pool.max_size)
    except Exception:
        log.exception("resize failed for generation %s", config.generation)
```

## Lifetime

A hook lives as long as its guard is open, and a configuration holds its
hooks — so a hook registered and forgotten runs for the life of the
process. That is fine for the ones a service sets up at startup, and a
leak for the ones a test or a request registers:

```python
with config.on_reload(record):
    ...                     # registered here
                            # and gone here, however the block ended
```

Hooks hold a **weak** reference back to the configuration internally, so
a hook never keeps a configuration alive; the reverse is not true, so a
closure that captures a large object keeps that object alive until the
guard closes.

## The whole surface, running

[`examples/16_callbacks.py`](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/16_callbacks.py)
runs all five shapes end to end, with a stand-in pool that records what
each hook cost it — including the handover to a thread that owns the
resource, and the async follower that needs no callback at all.

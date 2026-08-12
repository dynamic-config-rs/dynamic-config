# Web Frameworks

One rule carries every integration: **read `current()` once per request
and use that value for the whole request.** A reload landing halfway
through would otherwise show one request two configurations — the single
bug hot reload introduces if you let it.

The other rule is what *not* to do: do not copy configuration into the
framework's own settings object at startup. A copy never reloads, and
that is exactly the thing this library exists to fix.

## FastAPI

Configuration is a dependency, which is what makes it testable:

```python
config = DynamicConfig(Database, key="db").file("config.toml").env("APP_")
config.init()

app = FastAPI()

def current_config() -> Database:
    return config.current()          # one attribute lookup, no I/O

@app.get("/async/health")
async def health_async(db: Database = Depends(current_config)) -> dict[str, object]:
    # On the event loop: the read needs no await, so the handler never
    # yields just to see its own configuration.
    return {"host": db.host, "pool": db.pool_size}

@app.get("/sync/health")
def health_sync(db: Database = Depends(current_config)) -> dict[str, object]:
    # On a worker thread, which FastAPI uses for plain `def` endpoints.
    # The same read, and it is thread-safe: the model is immutable and
    # the swap atomic, so a reload landing mid-handler cannot tear it.
    return {"host": db.host, "pool": db.pool_size}
```

**Both endpoint styles read configuration identically**, which is the
point: there is no async variant of `current()` because there is nothing
to await. A dependency that *does* touch the sources — `explain`,
`check` — is real work and belongs off the loop:

```python
@app.get("/explain")
async def explain(path: str) -> dict[str, str]:
    return {"explanation": await asyncio.to_thread(lambda: str(config.explain(path)))}
```

Start the watcher in the app's lifespan:

```python
@asynccontextmanager
async def lifespan(app: FastAPI) -> AsyncIterator[None]:
    watch = await config.watch_async(debounce=0.25)
    try:
        yield
    finally:
        watch.stop()

app = FastAPI(lifespan=lifespan)
```

Two details, both deliberate.

`watch_async` rather than `watch`: the loop starting the app is the loop
that will answer its requests, and starting a watcher registers
directories with the notification backend — syscalls the calling thread
waits out. It is a fraction of a millisecond natively and single-digit
milliseconds when `poll_interval` makes it scan a large directory first.
This runs once, so the sync call would survive review; the async one
costs nothing to prefer. `stop()` needs no twin: it drops the backend
and returns without joining the thread or draining a debounce window.

One lifespan rather than a startup handler plus a shutdown handler:
start and stop are the same function, so the watcher cannot be started
without being stopped. That matters because a second `watch()` on one
configuration is `AlreadyExists`, deliberately — two watchers on one
file could only mislead — and an app gets instantiated more than once
(`uvicorn --reload`, a test suite building client after client). Paired
this way each run stops the last one's watcher before the next starts.

Overriding it in a test is FastAPI's own mechanism:

```python
app.dependency_overrides[current_config] = lambda: Database(host="localhost")
```

Importing a module should not begin filesystem work, which is why none of
this happens at import. `detach()` is the alternative to holding the
handle when the watcher should simply live as long as the process — the
binding stops it at interpreter shutdown either way.

The runnable version is
[`examples/10_fastapi_service.py`](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/10_fastapi_service.py).

## Flask

```python
@app.get("/health")
def health():
    db = config.current()            # inside the view, not in app.config
    return jsonify(host=db.host, pool=db.pool_size)
```

The habit worth breaking here is `app.config.update(...)` at startup:
values copied into `app.config` are frozen at the moment they were
copied. Reading through `config.current()` inside the view keeps reload
working and costs an attribute lookup — Flask does more than that
building the request object.

For a factory-style app, build the `DynamicConfig` next to the app and
close over it, or hang it on the app object (`app.extensions["config"]`)
so blueprints can reach it.

Runnable:
[`examples/11_flask_service.py`](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/11_flask_service.py).

## Django

Django's `settings` are read once at import and frozen — right for
`INSTALLED_APPS`, wrong for the handful of values an operator actually
turns during an incident. Split them:

```python
# settings.py
from dynamic_config import DynamicConfig

RUNTIME = DynamicConfig(Runtime, key="runtime").file("/etc/app/runtime.toml")
RUNTIME.init()
```

```python
# views.py
from django.conf import settings

def health(request):
    runtime = settings.RUNTIME.current()
    return JsonResponse({"pool": runtime.pool_size})
```

Django's static settings keep doing what Django needs; the reloadable
half is validated by Pydantic and can change without a restart. Start the
watcher from an `AppConfig.ready()` hook rather than from `settings.py`,
so it starts once per process and after the app registry is built.

Runnable:
[`examples/12_django_settings.py`](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-python/examples/12_django_settings.py).

## Workers and CLIs

The same rule with a different clock: read once per *unit of work*.

```python
def handle(job):
    settings = config.current()      # once per job
    ...
```

For a long-running consumer, `async for db in config.changes()` is the
shape that reacts to a change rather than polling for one — see
[Async & asyncio](async.md).

## Gunicorn, uWSGI and other pre-forking servers

Each worker process gets its own copy of everything, including the
watcher — which is fine and is what you want: every worker reloads
independently from the same file. Two cautions:

- Start the watcher **after** the fork (a `post_fork` hook, or a startup
  handler), never at import time in the master. A watcher thread does not
  survive `fork()` in the child, and the master's copy would be the only
  one left running.
- With `--preload`, the module-level `init()` runs once in the master and
  the loaded snapshot is inherited by the children, which is a *good*
  thing: one parse, N workers. The watcher still has to start per worker.

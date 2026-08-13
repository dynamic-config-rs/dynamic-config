# Implementation Details

How the binding is built, for anyone changing it — or deciding whether to
trust it. This page is what the code does, which is not always what the
design document that preceded it said; that document has been retired
now that every decision in it either shipped or was replaced by one
recorded here.

## The two halves

```text
              Python application code
                        │  attribute access, nothing else
                        ▼
            cached Pydantic model instance     ← swapped atomically per install
                        ▲
                        │  model_validate(dict), once per resolve
                        ▼
                 PyO3 boundary  (dynamic_config._core)
                        ▲
                        │  resolved tree → dict, no JSON detour
                        ▼
          dynamic-config instance engine (Rust)
   files · env · dotenv · profiles · strict_env · precedence
   watch · debounce · LKG cache · provenance · explain · check
```

**`dynamic_config._core`** is the compiled half: the engine, the value
conversion, and the one place Python is entered on a reload.
**`dynamic_config`** is ordinary Python around it — the generic
`DynamicConfig`, the decorator, the asyncio bridge, the secret
derivation. That split is deliberate: typing, introspection and event
loops are all clearer in Python, and none of them is on the read path.

## Where validation happens, and why it is there

The binding registers Pydantic validation as the engine's own `validate`
hook, which the loader calls **after deserializing and before installing
anything**. Everything else follows from that placement:

- A model Pydantic rejects never installs, so the previous snapshot keeps
  serving.
- The last-known-good cache is written *after* a successful install, so a
  configuration that fails validation never reaches it.
- Recovery from that cache goes through the same hook, so a cache that no
  longer validates does not resurrect.

Getting there needed one change in the Rust crate: `Builder::validate`
used to take a bare `fn` pointer, which cannot capture a Pydantic class.
It now takes a closure. A plain `fn` still coerces, so nothing that
existed before changed.

## Publishing a validated model, exactly once

The validate hook cannot install — it returns `Result<(), Error>` — so it
*stages* the model it built, and the install path publishes it. Two
paths arrive at that publish:

1. the engine's own `on_reload` hook, which fires for every install after
   the first;
2. the explicit call after `init()` or `reload()` returns, which is what
   covers the first install (the engine skips hooks there, deliberately —
   installing is not reloading).

Both call the same `commit`, and both can arrive for one install. A
sequence number stamped at validation makes the second a no-op: publish
once, bump the generation once, run the hooks once. Without it every
reload fired every hook twice, which is exactly what the test suite
caught.

If the staged tree does not match the tree being installed — a
concurrent `load()` staged something else in between — `commit`
validates the installed tree rather than publishing the wrong model.

## The read path

`current()` never crosses into Rust. Each published model is copied onto
the Python configuration object by a hook registered at construction, so
a read is `self._cached` and a `None` check.

The measurement that forced this: returning the model *from* Rust cost
251 ns against a module global's 20, because a PyO3 method call is
roughly ten attribute lookups. The Python-side cache measures 28 ns. The
hook holds a **weak** reference to the configuration — a strong one would
be a cycle through a `#[pyclass]`, which Python's collector cannot
traverse, so nothing would ever be freed.

Two caches mean they can disagree, so a test asserts they do not — after
init, reload, a watch-driven reload, `replace`, and recovery.

## Threads, and the GIL

- **Loading never holds the GIL.** Every call into the engine releases it
  (`py.detach`) and the validate hook re-acquires it for the convert-
  validate-swap step, which is microseconds.
- **The engine handle is cloned out of its lock before anything slow.**
  Holding a Rust mutex while waiting for the GIL is a deadlock: a second
  thread blocked on the mutex is a thread holding the GIL the first one
  needs. The threading suite found exactly that, and the fix was to make
  the engine an `Arc` that callers clone and use outside the lock.
- **Hooks run on the thread that reloaded.** A raising hook is reported
  through Python's unraisable channel and the rest still run — the
  crate's panic-isolation contract in Python's vocabulary.
- **Waits release the GIL** and are bounded (a quarter second per slice),
  so cancelling an `async for` is noticed promptly rather than at the
  next reload.
- **A Python remote source is called straight through**, not handed to a
  worker thread. `refresh_remote()` detaches for the whole refresh and
  the shim re-takes the GIL only to call `fetch()`; the design note that
  preceded the feature assumed that would stop the process, and the
  measurement says otherwise — a `fetch()` doing I/O releases the GIL
  itself, so a second thread keeps running at 68–102% of its free rate.
  The worker would also have created the one deadlock this shape does not
  have: a `fetch()` calling back into the extension would be waiting on
  the thread it is running on. See
  [Remote Stores in Python](remote-stores.md#the-gil-measured).
- **No lock is held across a fetch.** The engine clones its source out of
  the remote slot, and the shim clones the Python object out of its own,
  before either calls anything. That is what lets a `fetch()` read
  `current()`, `snapshot()` or `explain()` on the configuration it is
  fetching for. Calling `refresh_remote()` from inside one is refused by
  a thread-local flag, because that is recursion rather than re-entrancy.

## Interpreter shutdown

A watcher thread that outlives finalization would call into a Python that
is no longer there. The binding registers an `atexit` handler that stops
every live watcher and drops every cached model while the interpreter is
still whole; live watchers and configurations are tracked in weak sets so
this costs nothing and keeps nothing alive. Rust's `Drop` never touches
Python.

The suite runs this for real, in subprocesses: a detached watcher at
exit, an exit during a reload storm, a hook that reads back through its
own configuration, a configuration dropped while watching, and a process
exiting while a Python remote source is mid-fetch.

A Python remote source is the same hazard wearing a different hat, and it
needed one more move than the watchers did. The shim the engine calls
lives in a `Remote` that is leaked `&'static`, so a `Py<PyAny>` stored
*inside* it would be immortal — and since the ordinary shape of a source
is one that holds the configuration it feeds, that would be a cycle
running through a `static` no collector could reach. The object lives on
the configuration instead, behind a `Mutex`; the shim holds a `Weak` to
it. That makes it an ordinary edge: visited by `tp_traverse`, dropped by
`tp_clear`, and dropped again by the `atexit` release, after which a late
fetch answers *released* rather than calling into a torn-down
interpreter.

## Secrets

At construction the binding walks `model_fields` for `SecretStr` and
`SecretBytes` — through `Optional`, unions and nested models, as dotted
paths — and seeds the same secret list the generated Rust `builder()`
seeds. The names used are the ones a *file* could carry — **all** of
them: the field name and every alias Pydantic accepts, whether that is a
plain string, an `AliasPath`, an `AliasChoices` of either, or one an
`alias_generator` wrote. Deliberately generous, because the two errors
are not symmetrical: listing a name nothing supplies costs a key that
never appears, and missing one puts a password in `explain` and in the
"redacted" cache on disk. That was not hypothetical — the earlier rule
picked *one* name per field, and every other spelling leaked.

The same walk descends into Pydantic dataclasses, and treats a
`RootModel` as living where its *outer* field is rather than at the
`root` key no file writes.

That list drives everything downstream: the redacted cache drops those
paths, `explain` renders them `***`, and the scrubbed `ValidationError`
keeps locations and messages but not input values.

Nested secrets needed a second change in the Rust crate: both redaction
doors matched only the *head* of a path, so `credentials.password` was
redacted nowhere. `touches_secret` now answers one question for both —
is this path a secret, under one, or an ancestor of one.

## Value conversion

Both directions build the target structure directly. A JSON string round
trip would parse twice and lose the integer/float distinction — `port =
5432` must not arrive as `5432.0`, a `bool` must not arrive as `1`, and a
`u64` above `i64::MAX` must keep its digits. Each of those has a test.

Coming back the other way (`set_default`, `set_override`), anything
without a configuration meaning is refused at the call: a function, a
`NaN`, a dict with non-string keys. `SecretStr` is understood, because
there the caller is *supplying* the value.

## What the wheel contains

`cdylib` + PyO3 `abi3-py39`: one wheel per platform covers every
supported interpreter. The remote store crates are deliberately absent —
their clients would multiply the build matrix and ride into every wheel.
That is also why the wheel carries no tokio: the Rust `tokio` feature
routes the crate's *own* async loads into tokio's blocking pool, and this
binding never takes that path, because a Python loop can only await a
Python future.

## Versioning

The package versions independently of the Rust crates, and is excluded
from `cargo release`. It embeds the engine rather than depending on a
published version of it, so a Rust-only release has nothing in it for a
Python user. `dynamic_config.__version__` is the package;
`__engine_version__` is the crate it was built against.

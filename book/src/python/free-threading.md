# Free-Threaded CPython

**The wheel is declared free-threading-safe, and this page is the audit
behind the declaration** — every `static`, every place correctness rode on
the GIL, every shared Python object, with what the audit found and what it
changed. Two of its predictions measured false and two real defects came
out of it; both are described below.

The declaration is `#[pymodule(gil_used = false)]`, which sets
`Py_mod_gil = Py_MOD_GIL_NOT_USED` so a free-threaded interpreter does not
turn the GIL back on for the process at import. It is written out in
`src/lib.rs` even though PyO3 has made it the default since 0.28, because
a claim this size belongs in the source that makes it rather than in a
dependency's default — and because that default has already moved once, in
the other direction.

What proves it is `tests/test_free_threaded.py`, run by CI's
`python-free-threaded` job on CPython 3.14.0t: the whole suite, plus ten
further iterations of `test_threading.py`, `test_shutdown.py` and this
file. What it does not prove is at the bottom of the page.

## The wheel: one per interpreter, not one per platform

The ordinary wheel is `abi3-py39`, which is what makes one wheel per
platform cover every interpreter from 3.9 up. **A free-threaded build has
no stable ABI to target**: `Py_GIL_DISABLED` interpreters are not abi3, so
they need a *second* wheel per platform, built without abi3 and tagged
`cp314t`.

**The interpreter selects the ABI, not a build flag.** `maturin build -i
python3.14t` produces a version-specific `cp314-cp314t` wheel; without
`-i`, maturin builds abi3 against no interpreter in particular and never
looks at the one on `PATH`. That is measured, not assumed — and it means
the `-i` is the load-bearing part of the recipe.

`abi3` is nonetheless a Cargo feature the free-threaded build switches
**off**. Cargo features are additive, so nothing can turn abi3 off by
being turned on: it is a *default* feature that every ordinary build gets
by doing nothing, and the free-threaded wheel is built with
`--no-default-features`. That does not change the wheel's tag. What it
changes is that pyo3 is never *asked* for abi3, so the build does not lean
on a fallback pyo3's own authors label a backward-compatibility path —
with abi3 on, both maturin and pyo3 warn and fall back rather than fail.

**3.14t only, not 3.13t.** PyO3 0.29 dropped 3.13t, following CPython,
which promoted free-threading from experimental to supported in 3.14. A
`cp313t` wheel is therefore not buildable from this source, and the
release job builds one free-threaded interpreter rather than two.

**Linux only, for now.** The free-threaded wheel wave covers manylinux
x86-64 and aarch64. The macOS and Windows runners were not verified for a
free-threaded interpreter when this landed, and a release job that fails
on an unverified guess is worse than one platform building from source.
The job asserts its own wheel tags, so widening it is a matter of running
it once.

## The audit

### Every `static` in the compiled half

There are none. The engine's runtime layers are `&'static` *references*,
but each is a `Box::leak` made per configuration object rather than a
shared global — one `Layer` each for defaults, overrides and flags, one
`EnvBindings`, one `Aliases`, one `Remote`. None is reachable from any
other configuration, and every one of them guards its own contents with a
`Mutex` in the core crate rather than relying on the interpreter.

The one thread-local is `remote.rs`'s in-fetch flag, which is a per-thread
fact by construction and stays one without a GIL.

### Every `unsendable` `#[pyclass]`

There are none. All three classes — `Config`, `Watch`, `Snapshot` — are
`frozen`, hold their mutable state behind `Mutex`/`RwLock`, and are
therefore `Send + Sync`. Nothing here is pinned to the thread that made
it, which is the property `unsendable` exists to express and the one a
free-threaded build makes load-bearing.

### `commit()` — the staged sequence

`src/config.rs` claims the published sequence with `fetch_max` rather than
read-then-store, so two commit paths for one install cannot both win. That
was written pre-emptively for this item, and the audit's job was to check
it stayed that way. It did — and
`test_readers_and_reloaders_agree_under_real_parallelism` now exercises it
where the GIL is not doing the serialising for it.

### `Inner::validate` — the staged slot

Last-writer-wins, deliberately. Two concurrent loads can both stage, and
the loser's model is dropped rather than published — because both `load()`
and `commit()` compare the staged **tree** against the one they are acting
on and re-validate when it does not match. That comparison is what makes
the slot safe to share; it is not new, and it does not depend on the GIL.

### The hook list

`hooks: Mutex<Vec<(u64, Py<PyAny>)>>`. The design note expected the lock to
be held while hooks run, which would make a hook that registers or
unregisters another one a re-entrant lock — a deadlock, not a panic.
**Measured: it is not held.** `run_hooks` clones the list out of the lock
before running any of it, and
`test_a_hook_that_registers_another_hook_does_not_deadlock` and its
self-unregistering twin assert that on every build. Both pass under the
GIL, which is where a regression would first appear.

### `__traverse__`

`try_lock`, returning `Ok(())` when the lock is held — the right shape, and
more so on a free-threaded build, where the collector runs concurrently
with Python code rather than between bytecodes. A traverse that blocked on
a lock a running hook held would stop the collector. The same rule covers
the Python remote source, which is visited the same way, and `__clear__`
drops both edges.

### `_LIVE_CONFIGS` / `_LIVE_WATCHES`

**Changed by this audit.** `weakref.WeakSet` is only as atomic as the GIL
makes it: its `add` is several bytecodes over an internal set plus a
pending-removals list, and configurations are built from many threads in
this suite alone. A registry that dropped entries would leave watchers
running into finalization, which is the crash the whole `atexit` sweep
exists to prevent. Both sets are now guarded by one `threading.Lock`, held
around the mutation and around the snapshot the sweep takes — never while a
watcher is being stopped.

### The Python remote source

The object lives in the configuration rather than in the shim the engine
holds, behind a `Mutex`, and is cloned out of that lock before Python is
entered. The in-fetch guard is a thread-local. Nothing about it assumes one
thread.

## The read path, measured

`current()` never crosses into Rust: it is a Python attribute lookup on the
configuration object, kept fresh by a hook. Nothing on it clones a
`Py<PyAny>`, so the "every clone is a real atomic" cost of a free-threaded
build should not land there. That was the structural argument; here is the
measurement, from `benchmarks/read_path.py` on both interpreters:

| Interpreter | `config.current()` | a module global | ratio |
|---|---|---|---|
| CPython 3.14.6, GIL | 29 ns | 21 ns | 1.4× |
| CPython 3.14.0t, no GIL | 27 ns | 20 ns | 1.35× |

**Read the ratio, not the nanoseconds.** The two interpreters are different
patch releases, the machine was doing other work, and the spread across
runs of the *same* interpreter reached 29–46 ns — larger than any
difference between the two. What survives that noise is the column that
normalises it away: a read costs the same multiple of a plain attribute
lookup on both builds. Free-threading does not put anything extra on the
read path, which is what the structure predicted.

## What is tested

`tests/test_free_threaded.py` holds two kinds of test. The ones that run
everywhere are places the audit found correctness riding on the GIL
without saying so — each a latent bug on a GIL build too. The ones guarded
by `Py_GIL_DISABLED` need real parallelism to mean anything.

| Test | Runs |
|---|---|
| a hook that registers another hook does not deadlock | everywhere |
| a hook that unregisters itself does not deadlock | everywhere |
| configurations built from many threads are all registered | everywhere |
| readers and reloaders agree under real parallelism | on 3.14t, in CI |
| the module declares itself GIL-free | on 3.14t, in CI |

The last one asserts `sys._is_gil_enabled()` is false after importing the
extension. An earlier version watched for the interpreter's warning
instead and passed whether or not the declaration was there — the warning
is emitted once per process at the first import, so reloading the module
and catching warnings catches nothing. A gate that cannot fail is not a
gate, which is the whole reason this page exists.

## What this still does not prove

- **One interpreter, one platform.** 3.14.0t on x86-64 Linux. The races
  that matter are timing-dependent, and a different core count or memory
  model can surface one this machine never will.
- **Ten iterations is evidence, not proof.** Most of these races need
  contention to appear. The ten-iteration loop under load is the floor,
  and a green run of it is the absence of a failure rather than the
  presence of a guarantee.
- **The engine's own concurrency is argued, not model-checked, from
  Python.** `loom` and `shuttle` cover the Rust side's fence and wake
  protocol; nothing model-checks the binding's staged-slot protocol
  through a Python interpreter.
- **The remote wheel is abi3 only.** `dynamic-config-py[remote]` has no
  free-threaded build, so a 3.14t install cannot take the remote stores
  with it.

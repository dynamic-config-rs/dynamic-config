# Python bindings: the implementation plan

The reference document for `dynamic-config-python` — a PyO3 extension that
pairs this crate's configuration runtime with Pydantic's validation. It
exists because the split is genuinely good: Rust owns sources, layering,
watching, recovery and provenance; Pydantic owns the schema and the
validators Python people already have; Python code reads a cached, typed
snapshot for the price of an attribute lookup.

This plan reflects an external design review plus this repository's own
decisions. Where the two differ, the decision and its reason are both
here. Nothing in it is built yet; the [ROADMAP](../ROADMAP.md) tracks when.

## Goals and non-goals

**Goals**

- Arbitrary Pydantic models as the schema — `field_validator`,
  `model_validator`, aliases, `SecretStr`, custom types, all of it.
- The full lifecycle Python-side: `init`, `watch`, `reload`, `current`,
  `on_reload`, `async for changes`, last-known-good recovery,
  `source_of` / `explain` / `check`.
- The crate's semantics survive the boundary intact: a bad reload —
  including one Pydantic rejects — keeps the previous snapshot serving; a
  reader never pays for a reload; values stay out of every diagnostic
  except `explain`.
- `current()` cost: one Python attribute lookup on a cached object. No
  per-read boundary crossing, no per-read validation, ever.

**Non-goals (v0)**

- The remote store crates. Their clients (tonic, aws-sdk, …) would ride
  into every wheel; remote support returns later as opt-in features or
  separate wheels. The `RemoteSource` trait is not exposed to Python in
  v0 either — a Python-implemented source would put Python on the fetch
  path, which deserves its own design pass.
- `save`/`schema` — Pydantic already serializes and emits JSON Schema;
  duplicating that through Rust helps nobody.
- Direct `pydantic-core` coupling. `model_validate` is the public,
  stable entry point; internals are version-churn we do not want to
  chase. Revisit only if profiling ever shows validation as a real cost —
  reloads are rare, so it will not.

## The architecture

```text
              Python application code
                        │  attribute access, nothing else
                        ▼
            cached Pydantic model instance     ← swapped atomically per reload
                        ▲
                        │  model_validate(dict), once per reload
                        ▼
                 PyO3 boundary
                        ▲
                        │  figment Value → PyObject, no JSON detour
                        ▼
          dynamic-config instance engine (Rust)
   files · env · dotenv · profiles · strict_env · precedence
   watch · debounce · LKG cache · provenance · explain · check
```

Rust resolves; Pydantic validates; Python reads a cache. Validation runs
exactly once per successful resolve, never per read.

## Part 1 — core changes (in `dynamic-config`, useful to Rust too)

The engine is 80–90% ready. Two assumptions block instance use, and both
fixes stand on their own merits:

### 1a. The instance engine: `Dynamic<T>`

`Builder<T>` currently installs through `Option<fn(T)>` — a fn pointer,
because the generated `builder()` points it at a `static` cell. An
instance needs its own storage:

```rust
pub struct Dynamic<T> {
    cell: Arc<ConfigCell<T>>,
    builder: Builder<T>,
    id: ConfigId,                  // see 1b
}
```

with `current() -> Option<Arc<T>>`, `init`, `reload`, `watch`,
`on_reload`, `changes`, and the diagnostics delegating to the builder.
Internally the installer generalizes from `fn(T)` to a small enum
(`Static(fn(T))` | `Cell(Arc<ConfigCell<T>>)`) rather than
`Arc<dyn Fn>` — two known shapes, no allocation on the static path, and
the generated code keeps compiling byte-for-byte.

This is not Python plumbing: `Dynamic<T>` is the answer to every Rust
user who asked for two configurations of one type (multi-tenant, tests,
one process serving two files). It ships as public API with its own
chapter, tests and bench row, independent of the bindings.

### 1b. Watch identity beyond `TypeId`

The watcher registry is `BTreeMap<TypeId, &'static str>` — right for
types, meaningless for instances (every `Dynamic<serde_json::Value>` is
the same type). The key becomes:

```rust
enum WatchKey { Type(TypeId), Instance(u64) }   // u64 from an AtomicU64
```

Type-keyed behaviour is unchanged (same one-watcher-per-type contract,
same `AlreadyExists`); an instance gets one watcher per *instance*, and
its `Drop` frees the key exactly as the handle does today.

### 1c. Value export

`Snapshot::values()` is `pub(crate)`. The boundary needs the resolved
tree; the export is a `Snapshot::to_value(&self) -> figment::value::Value`
(or an iterator the converter walks) — figment types stay out of the
signature if a small owned mirror proves nicer. Decided at
implementation time; the constraint is *no JSON string round trip*.

## Part 2 — the binding crate (`dynamic-config-python`)

Workspace member, `publish = false` until it graduates (the CLI's
pattern). `cdylib` + PyO3 `abi3-py39`, built and released with maturin;
wheels for the usual manylinux/macOS/Windows matrix, `pip install
dynamic-config` (PyPI name claimed early, like the crates were).

### The Python API

Two doors, one engine — the same doctrine as Rust's attribute/builder
split, translated:

**The class API** (explicit, the reference surface):

```python
from pydantic import BaseModel
from dynamic_config import DynamicConfig

class Database(BaseModel):
    host: str
    port: int = 5432

config = (
    DynamicConfig(Database, key="db")
    .file("config.toml")
    .env("APP_")
    .strict_env()
    .cache("/var/lib/app/last.json", mode="redacted")
)
config.init()
watch = config.watch(debounce=0.25)

db = config.current()          # a Database instance, cached
```

**The decorator** (sugar over the same instance, for the
pydantic-settings crowd):

```python
from dynamic_config import dynamic_config

@dynamic_config(key="db", files=["config.toml"], env="APP_", watch=0.25)
class Database(BaseModel):
    host: str
    port: int = 5432

Database.current()             # classmethods attached by the decorator
Database.config                # the underlying DynamicConfig, for the rest
```

The decorator constructs a `DynamicConfig`, stores it as
`Database.config`, and attaches `current`/`reload`/`source_of`/`explain`
classmethods. It does **not** call `init()` — import time is the wrong
time to read files; `Database.config.init()` (or an explicit
`init=True` kwarg for scripts that want it) stays a deliberate act. A
second decoration of the same class is an error, mirroring
`AlreadyExists`. In Python, runtime-configured decorators are idiomatic
where Rust's argument-free attribute was not — the *engine-level* rule
(declaration separate from a configurable builder) holds in both.

Full surface, mirroring Rust one-to-one where it exists:
`init/init_async`, `load/load_async` (validate-don't-install),
`reload`, `watch/watch_poll`, `current/try_current`, `replace(model)`,
`on_reload(fn)` (returns a guard; `with config.on_reload(...)` works),
`changes()` (async iterator), `source_of`, `is_set`, `snapshot()`
(a mapping + `source_of`), `explain` (object with `rows` and `__str__`
as the table), `check()`, `set_default/set_override/set_defaults/
set_assignments/clear_*`, `alias`, `bind_env`, `diff`/`changed_paths`.

### Validation flow, exactly

```text
reload trigger (watcher / reload() / init)
    → Rust: load + merge + strict checks     (no GIL)
    → Rust: resolved tree → PyObject         (GIL, microseconds)
    → Python: Model.model_validate(dict)     (GIL, once)
    → ok:  swap the cached Py object + bump generation → hooks, wakers
    → err: ValidationError logged (paths only — see secrets), previous
           snapshot keeps serving; init() raises instead, as Rust errors do
```

The LKG path is identical: recovery output goes through
`model_validate` too, exactly as Rust recovery goes through
`.validate(f)`. A cache that deserializes but no longer validates does
not resurrect.

### The GIL strategy

Loading and watching never hold the GIL; Python is entered only at the
convert-validate-swap step and inside user hooks.

- **Default: validate on the watcher thread.** Reloads are rare; a brief
  `Python::attach` per reload is simple, keeps the failure on the reload
  path where it belongs, and makes `current()` unconditionally cheap.
  Hooks run there too, each wrapped: a raising hook is logged and the
  rest still run — the Rust panic-isolation contract, translated.
- **Rejected: lazy validation on first `current()` after a change.** It
  moves failure (and latency) onto a reader; the whole design exists so
  readers never pay.
- **asyncio-native alternative:** `async for model in config.changes()` —
  the Rust `Changes` future bridged to an awaitable; a service that wants
  validation on its loop drives it there and calls `replace()` itself.
- **Free-threaded CPython (3.13+):** the engine is already lock-free on
  reads; the binding declares free-threaded support only after a
  dedicated audit of the convert/validate/swap step, not by default.

### Secrets across the boundary

Nobody re-declares secrets. At construction the binding walks
`Model.model_fields` and derives the secret list from the types Python
already uses — `SecretStr`/`SecretBytes` (nested models included, as
dotted paths) — and seeds it exactly as the generated Rust `builder()`
seeds `with_secrets`. Everything downstream then just works: the
redacted LKG drops those fields, `explain` returns them as `***`,
`ValidationError` output is passed through a scrubber that keeps paths
and messages but not input values (Pydantic's own errors can echo the
offending input — that is the one place its defaults and this crate's
rules disagree, and this crate's rule wins). The planted-secret test
suite runs against every one of these doors from Python.

### Performance model and targets

- `current()`: return the cached `Py<PyAny>` — one refcount bump. Target
  and test: within noise of reading a module-global; regression-checked
  with `pytest-benchmark`.
- Reload cost: resolve (Rust) + convert + validate, once. Convert is a
  direct figment-tree → dict/list/scalar build; no `serde_json::to_string`
  → `json.loads` detour, which would double-parse and lose integer/float
  distinctions.
- Memory: the previous model drops when the swap succeeds and every
  reader's reference dies — Arc semantics on the Rust side, refcounts on
  the Python side; a leak test loops a thousand reloads under
  `tracemalloc` and asserts a flat profile.

### Zero-bug discipline

The bar is the Rust suite's, ported:

- **A pytest suite mirroring the Rust integration tests** — layering and
  precedence, strict_env (including recovery), LKG in all three modes
  (Fingerprint never recovers), watch with generous margins, hooks
  (including a raising hook not stopping the rest), decorator
  double-registration, explain redaction, provenance.
- **Planted-secret tests from Python** for every diagnostic surface,
  including `repr()` of every exposed object.
- **Threading tests**: N reader threads through a reload storm assert no
  torn reads and monotone generations; watcher + manual `reload()` racing.
- **GIL-safety**: hooks that themselves call back into `config.*`;
  `changes()` cancelled mid-await; interpreter shutdown while a watcher
  thread lives (the drop path must not touch Python after finalization —
  this is the classic embedding bug, and it gets a test).
- **Refcount/leak checks** in CI (the tracemalloc loop above, plus
  debug-build assertions).
- **CI**: a `python` job — maturin build, pytest matrix over supported
  CPython versions, `mypy --strict` against the shipped stubs. The wheel
  build itself becomes part of `publish-dry-run`.

### Developer experience

- **Type stubs** (`.pyi`) shipped and CI-checked: `DynamicConfig` is
  `Generic[M]`, `current() -> M`, so editors and mypy see `Database`, not
  `Any`.
- **Errors**: one `DynamicConfigError` hierarchy mirroring `ErrorKind`;
  Pydantic's `ValidationError` passes through untranslated (scrubbed) —
  it is the error Python users already know how to read.
- **Docs**: a book chapter (`Python bindings`) with the FastAPI pattern —
  read `current()` once per request — plus the decorator, asyncio
  `changes()`, and the boundary rules (what validates when, what the GIL
  strategy is, what is not exposed and why).
- **Repr discipline**: every binding object's `repr` follows the crate's
  Debug rules — shape, never values.

## Part 3 — the phases

1. **Core:** `Dynamic<T>`, `WatchKey`, value export — with Rust tests,
   book section, changelog. Ships in a normal release; Python not
   mentioned yet.
2. **Binding alpha:** crate + class API, files/env/profiles/strict,
   init/load/current/reload, secrets derivation, stubs, pytest
   foundation. In-repo, unpublished.
3. **Lifecycle:** watch, hooks, `changes()` asyncio bridge, LKG,
   overrides/defaults/aliases/bindings, explain/check/source_of,
   decorator. The GIL and threading test batteries land here.
4. **Hardening:** leak/refcount CI, benchmark suite, free-threaded audit,
   docs chapter, wheel matrix in dry-run.
5. **Publish:** PyPI name, maturin release wired into the release
   pipeline as its own wave, versioned in lockstep with the workspace.

Each phase ends inside the existing gates: fmt, clippy at both extremes,
the full Rust suite untouched-and-green, plus the growing Python matrix.

## Risks, named

- **Interpreter shutdown vs. watcher threads** — the highest-severity
  class of embedding bug; addressed by never touching Python from `Drop`
  and a dedicated shutdown test.
- **Pydantic major versions** — v2 only at first; the `model_validate`
  boundary is narrow enough that a v3, when it comes, is a contained
  change.
- **Wheel size and build matrix** — abi3 keeps it one wheel per
  platform; remote stores stay out precisely to keep it that way.
- **Two sources of truth for "what is secret"** — avoided by deriving
  from the model rather than accepting a second list; the field-type walk
  must handle nesting, `Optional`, and unions, and its tests must plant
  secrets at every one of those shapes.

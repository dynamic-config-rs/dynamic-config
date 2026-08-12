# Roadmap

What is not in the crate yet and might be. Everything that shipped is described
in [README.md](README.md); what will *not* be built, and why, is under
[Not planned](book/src/limitations.md#not-planned) there.

Tags: **[viper]** exists in Go's [Viper](https://github.com/spf13/viper) and does
not here. **[figment]** is something the underlying loader,
[figment](https://docs.rs/figment), can do that this crate does not expose.
**[own]** is neither — an idea from using the thing.

<!-- Keep this shape. An item earns a place here when it has a *decision* in it:
     what the alternatives are, and why one of them is not obviously right. An
     item that is only "add X" belongs in an issue. -->

---

## The next release

Decided, not aspirational — each item keeps its full description in its
own section; this is only what ships now. The shape follows
[the 1.0 doctrine](#the-road-to-10-is-stabilisation-not-features-own):
0.3 stabilised what existed, 0.4 built the instance engine and the
evidence for it (both lists shipped whole; the story is the changelog's),
and 0.5 spends that engine on the thing it was built for.

**0.5 — the Python bindings.** One release, one subject. The plan this
was built from has been retired now that all of it shipped; what the code
does and why is
[Implementation Details](book/src/python/internals.md). What ships:

- `dynamic-config-python` — a PyO3 extension module — *shipped*: the
  class API (`DynamicConfig(Model, key=..)`), the decorator, the full
  lifecycle (init, load, reload, watch, hooks, `async for` changes,
  last-known-good recovery) and the diagnostics (`source_of`, `is_set`,
  `explain`, `check`, `snapshot`).
- **The schema owns validation, Rust owns resolution.** A model is
  validated exactly once per successful resolve and cached; `current()`
  is an attribute lookup, never a boundary crossing. A rejected reload
  keeps the previous snapshot serving, exactly as a Rust `validate`
  refusal does. The schema can be a `dataclasses.dataclass` — the base
  install has **no dependencies** — a Pydantic model, a Pydantic
  dataclass or a `BaseSettings` class; `[pydantic]`,
  `[pydantic-settings]` and `[all]` buy the ones that need a library.
- **Secrets are derived, not re-declared** — the binding walks
  `model_fields` for `SecretStr`/`SecretBytes` and seeds the same secret
  list the generated Rust `builder()` seeds, so the redacted cache,
  `explain` and the scrubbed `ValidationError` all follow from the model
  the Python author already wrote.
- **Type stubs**, a pytest suite that mirrors the Rust integration tests
  (layering, strict env, LKG in three modes, watch, hooks, threading, GIL
  and interpreter-shutdown safety, planted secrets), and a CI job that
  runs them — *shipped*, a hundred and ninety-four tests over six
  interpreter versions (3.9 through 3.14), with `mypy --strict` and
  `ruff` over the package and every example run as part of the same job.
  Three suites sit above the unit ones: `test_pydantic.py`, which asserts
  that whatever a Pydantic model may be it may be here — inheritance,
  `model_config`, validators, all four alias shapes, `RootModel`,
  Pydantic dataclasses, generics, discriminated unions and
  `BaseSettings`; `test_dataclasses.py`, which pins what the
  dependency-free schema checks and what it refuses; and
  `test_integration.py`, which runs whole scenarios and drives the
  shipped framework examples, so an example that rots fails the suite. A
  separate job installs the wheel into a bare virtualenv and proves the
  base install needs nothing.
- **The read path is an attribute lookup** — *shipped*: 1.1× a module
  global, because the model is published into the Python object rather
  than fetched back across the boundary, against 34× for a per-read
  validation. The nanoseconds and the machine they were measured on are
  in the same table ([the chapter](book/src/python.md#what-a-read-costs));
  what the design claims is asserted exactly rather than timed.
- **PyPI as `dynamic-config-py`.** The bare name is taken by an unrelated
  single-release package from 2022; the distribution takes the qualified
  name and the import stays `dynamic_config`, which is what every example
  in the book and the plan spells. Reclaiming the bare name through
  PEP 541 is worth doing and is nobody's blocker.

Four things the wave found on the way, and fixed where they were wrong
rather than around:

- `Builder::validate` now takes closures — a validator that needs context
  could not be a `fn`, which is exactly what a binding needs.
- **Nested secrets were redacted nowhere.** A dotted secret path was
  missed by both the `explain` redaction and the redacted cache, which
  mattered the moment a model could nest.
- **A secret under an alias was redacted nowhere either.** The derived
  list held one name per field, so a file spelling it any of the other
  ways Pydantic accepts — `AliasChoices`, `AliasPath`,
  `populate_by_name`, a Pydantic dataclass — put the value in `explain`
  and in the "redacted" cache on disk. The list now holds every name a
  file could use, because over-listing costs a key nothing supplies and
  under-listing costs a secret.
- **`bind_env` could not see a `.env` file.** A binding names one
  variable exactly; a deployment that writes that variable into a `.env`
  file rather than exporting it means the same thing by it, and got
  nothing. Bindings now fall back to the `.env` files, below the real
  environment — the order those layers were already in.
- **A model holding an enum, a date or a `Decimal` could not be diffed.**
  `changed_paths` — the audit half of a reload — raised a `TypeError`
  for any schema with one of those in it, because neither `model_dump()`
  nor `dataclasses.asdict` unwraps an enum and none of them is a JSON
  scalar. And a *native TOML date* reached Python as a one-key marker
  dict, so a `date` field met a table and every schema refused it. Both
  convert now, in the one place the tree crosses the boundary.

### What 0.5 deliberately left out

Each of these was a **non-goal for v0**, and each is here rather than in
a footnote because "not yet" and "not ever" are different answers:

- **The remote stores in the wheel** *(0.6, demand-driven)*. etcd,
  Consul, Vault, NATS, Redis, S3 and Firestore are a gRPC stack, the AWS
  SDK and three HTTP clients between them. A wheel is built per platform,
  so every one of those would ride into every wheel for every user —
  including the ones reading a single TOML file. The shape when it comes
  is an opt-in wheel (`dynamic-config-py[etcd]`, its own build), not a
  flag on this one.
- **A tokio runtime in the wheel** *(follows the stores, not on its own)*.
  The Rust `tokio` feature routes the crate's *own* async loads into
  tokio's blocking pool; this binding never takes that path, because a
  Python loop can await a Python future and nothing else. Enabling it
  today would add a runtime no code enters. What answers the same
  question — which pool pays for the blocking half — is `set_executor`,
  which shipped. The async store clients are the one thing that would
  make a tokio build mean something.
- **`RemoteSource` implemented in Python** *(needs a design pass)*. A
  Python object on the fetch path means the GIL is held across a network
  call and a Python exception has to become a Rust error somewhere
  sensible. Both are solvable; neither is solvable casually.
- **Pydantic as a hard dependency** *(reversed — it is now optional)*.
  The base install has none at all: a `dataclasses.dataclass` is a schema,
  validated structurally, and `pip install dynamic-config-py[pydantic]`,
  `[pydantic-settings]` or `[all]` buy the other kinds. Importing the
  package with Pydantic uninstalled loads no Pydantic module, and CI
  asserts that in a bare virtualenv rather than in a sentence.
- **Direct `pydantic-core` coupling** *(refused)*. `model_validate` is the
  public, stable entry point. Binding to internals would be version churn
  for a cost that profiling says is not there — reloads are rare, and the
  read path does not validate at all.
- **`save` and JSON Schema from Python** *(refused)*. Pydantic already
  serializes models and emits JSON Schema, better than a second
  implementation would.
- **Encrypted files** *(blocked on a Rust trait)*. `Decryptor` is a Rust
  trait with no Python side, and shipping `age` to make one usable would
  put a crypto stack in every wheel for a door only Rust can open.
- **Free-threaded CPython support** *(0.6)*. The read path is lock-free
  and the binding's state sits behind ordinary locks, so there is no
  particular reason to expect trouble — but that is not an audit of the
  convert-validate-swap step, and declaring support without one is a
  promise made on optimism.
- **A `pydantic-settings` source shim** *(refused — but support shipped,
  the other way round)*. Wiring in as a `PydanticBaseSettingsSource`
  would inherit that library's lifecycle — read once, at construction —
  and lose the reloading that is the point. So the support goes the
  other direction: a `BaseSettings` class is a schema like any other
  model, and `DynamicConfig.from_settings(...)` reads its
  `SettingsConfigDict` and rebuilds the declaration as engine sources
  (`toml_file`/`json_file`/`yaml_file` become files, `env_file` becomes
  the dotenv layer, `env_prefix` becomes a binding per leaf field so
  `APP_PORT` stays `APP_PORT` rather than becoming `APP_<KEY>_PORT`).
  What has no engine equivalent — `secrets_dir`, `cli_parse_args`, an
  overridden `settings_customise_sources` — is refused at the call
  rather than dropped, and a settings class used *without*
  `from_settings` warns if it declares sourcing, because an `env_prefix`
  that silently reads nothing is the failure mode worth spending a
  warning on. A `secrets_dir` equivalent — a directory of single-value
  files, which is how Docker and Kubernetes mount secrets — is a
  reasonable engine source to add later; it is the only translation this
  had to give up.

The Python chapter's [Limitations](book/src/python/limitations.md) says
the same things to a user rather than to a maintainer; this list is the
one that decides what a later release picks up.

**0.6 and later, pulled by demand:** free-threaded CPython support for
the wheels (after that audit), remote stores as an opt-in Python extra
with the tokio runtime they need, `RemoteSource` from Python once its
design pass happens, `proc_macro_crate` rename, key
aliases across sections, `WriteDurability`, the embedded no-alloc wait
queue, the shared auth core and its dependents (`with_timeout` symmetry,
`ErrorKind::Auth`), runtime-agnostic S3 sleep, multi-key remote
documents, an eighth store when somebody asks, instruction-count
benchmarks, fuzzing harnesses, serde_yaml's future as upstream decides
it, and — far out, designed in the open first — the config server.

## Layers

### Key aliases across sections **[viper]**
`alias("pool.size", "pool.max_size")` moves a path within one section. A value
that moved *between* sections — `server.timeout` becoming `http.timeout` — is
not expressible, because a `LoadSpec` resolves one section at a time.

Doable by resolving the other section during the alias pass. Unclaimed, and
worth a real case first: the cost is that a load then depends on a section the
type does not own.

---

## Remote stores

### Reading several keys as one document **[own]**
Every store crate reads one key. A deployment that splits configuration across a
prefix — `myapp/db`, `myapp/server` — installs one source per section, which
works and is a little tedious.

Merging a prefix into one document is easy for etcd, Consul and Redis, awkward
for Vault and Firestore, and needs an ordering between keys defined before it
means anything. Possible, unclaimed.

### A store nobody has asked for yet **[own]**
`RemoteSource` and `AsyncRemoteSource` are public, so a new store is a crate
rather than a patch to this one. Seven exist; an eighth is worth adding when
somebody wants it, not before.

Each is a client dependency, a container in CI, an authentication story and a
set of failure modes to get right — the seven that exist took that seriously,
and an eighth done casually would be worse than none.

---

## The longer arc

### Instruction counts, not just wall clock **[own]**

The criterion suite that landed in 0.4 measures time, which a shared
runner measures badly — that is why the bench job is not a regression
gate. iai-callgrind counts *instructions* instead, which is stable enough
to gate on; the cost is valgrind on the runner and a second harness to
keep honest. Worth it the first time a performance regression gets
through the eye test. Cross-library comparisons stay out of CI either
way, and in a written-up experiment — they rot too fast to gate on.

### A config server **[own]**

The other half of the distribution story, in the spirit of Spring Cloud Config Server:
a small service that owns the files (or fronts a store), serves resolved
sections over HTTP, and pushes changes to subscribed clients — so a fleet
of services shares one source of truth without each carrying store
credentials. The client side is already here (`RemoteSource` + a watch
loop); the server would be a new crate with its own threat model (authn,
who may read which section, audit). Far future, and worth designing in the
open before building.

### The road to 1.0 is stabilisation, not features **[own]**

Two releases in two days is a build phase, not a track record, and the
API surface is now wide enough that its cost compounds. Before 1.0: a
deliberate quiet period — 0.3 was the API-review release (the figment
leak pass; both of its fixes shipped in 0.4, and its keep-decisions live
in the stability-tiers chapter and the builder tour), 0.4 froze the shape
of the engine, and 0.5 spends it on the bindings rather than widening the
Rust surface again. Then: real external users, then a freeze candidate.
New capability proposals queue behind stability during that window. The problem worth
solving by then is not a missing feature; it is that nothing this
sophisticated has been beaten up by strangers yet.

### A real no-alloc wait queue for the embedded crate **[own]**
`ConfigCell<T, const WAITERS>` sizes the parking lot, but N > WAITERS still
degrades to wake-churn (documented). An intrusive list would fix it without
an allocator; it also drags `unsafe` into a crate that forbids it. That
trade deserves its own design pass.

### Fuzzing the parsing surfaces **[scorecard]**

proptest already fuzzes the parsing surfaces on stable, but a property test
is not what the ecosystem's tooling recognises as fuzzing: OSSF Scorecard
scores the project zero on it. `cargo-fuzz` harnesses over the same
surfaces — the `.env` parser, units, the redaction walker, the section
mapper — would be recognised, and coverage-guided input generation does
find what proptest's random generation does not. Demand-driven: the
property tests carry the correctness argument today.

### `WriteDurability` as API **[own]**
0.1.0 fsyncs every atomic write, unconditionally. If someone measures real
pain from that, a `Normal`/`Fsync` mode is the escape hatch — not before.

### Shuttle, for what loom cannot reach **[own]**
loom (landed in 0.3) proves the remote fence and the wake protocol. The
residue is structural: group reload and the watch registry lean on
process-wide statics, which loom's iteration model does not tolerate, and
`ConfigCell` sits behind `arc-swap`, which loom cannot instrument.
Shuttle runs real code unmodified and is the tool to revisit for those.

### `proc_macro_crate` rename support **[own]**
`::dynamic_config` is hardcoded in the expansion, so renaming the dependency
breaks. `proc-macro-crate` fixes it at the cost of a parsing dependency in
the macro crate.

### serde_yaml's future **[own]**
Archived upstream (`0.9.34+deprecated`); figment pulls it regardless, so a
local switch buys nothing. Track figment; move when it moves.

### A shared auth core for the HTTP stores **[own]**
Consul, Vault and Firestore now share the margin (`REFRESH_WITHIN`) but
still triplicate the Session/Token machinery. Reconcile the semantics on
paper first; extract second.

### `with_timeout` symmetry across stores **[own]**
The three ureq crates take `with_timeout`; etcd/NATS/S3 configure timeouts
through their clients' own vocabulary. Either add pass-throughs or document
the asymmetry per README — decide once someone actually trips on it.

### Runtime-agnostic S3 watch sleep **[own]**
Blocked on the AWS SDK itself being tokio-bound; revisit if smithy's
runtime abstraction ever makes executor-independence real.

### `ErrorKind::Auth` **[own]**
The stores classify 401/403 internally now; a public variant would let a
caller treat "credentials are wrong" as a program-visible state. Decide the
boundary with a real consumer in hand.


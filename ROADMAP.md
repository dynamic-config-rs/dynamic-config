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

Decided, not aspirational — each item keeps its full description in its own
section below; this is the list and the order. The shape follows
[the 1.0 doctrine](#the-road-to-10-is-stabilisation-not-features-own): 0.3
stabilised what existed, 0.4 built the instance engine, 0.5 spent it on the
Python bindings, and **0.6 is the clearing release** — everything that had
been waiting for a reason other than "nobody has asked", done in one wave so
the surface stops accumulating.

**0.6 is built and unreleased.** What landed is not here: the changelogs
carry it, [README.md](README.md) describes the crate as it is, and this
file keeps only what is still open. Three of its items were answered by
*measurement* rather than by code, and each answer is written where it
belongs rather than here:

- **figment as a plug-in** was rejected. `Source` is already the name of the
  layer descriptor, `Source::provider` is already the trait the item asks
  for, and a parser plug-in has no `Origin` to report — so the seam that
  landed is narrower and public (`Value::parse`/`merge`/`render`/
  `overlapping_paths`), and the reasoning is in
  [the book](book/src/limitations.md).
- **The embedded wait queue** was rejected on the target's own numbers. Past
  the waiter budget the failure is a livelock rather than wake-churn, so
  raising the default only relocates the cliff; an intrusive node costs more
  RAM per waiter than a slot does, before the `unsafe`. What shipped is
  `waiter_evictions()` — the budget reports when it is set wrong.
- **A type-state builder** was rejected with a prototype: at the 1.71 floor
  `#[diagnostic::on_unimplemented]` does not exist, the error a user gets is
  worse than today's sentence, and the state parameter leaks into four
  public signatures.

**0.6 is complete.** Every item in it either landed or was answered by
measurement; the entries below are what those answers left behind, and none
of them is a design question any more.

1. [Instruction counts](#instruction-counts-not-just-wall-clock-own) — the
   harness and the workflow landed and have **never executed**: there is no
   valgrind where they were built. The gate arms itself the moment a
   maintainer runs the workflow once and commits the baseline it uploads.
2. [What a remote fetch does not yet report](#what-a-remote-fetch-does-not-yet-report-own)
   — the door exists now (`RemoteSink::failed`); what is left is one call at
   each failure site in the seven network watch loops.

Two 0.6 answers are worth keeping in view because they will be asked again:

- **The config server carries no OpenTelemetry SDK.** Four dependency trees
  and a background exporter in the one program holding every service's
  secrets is weight that has to earn its place, and this does not: the
  library side is free — `tracing` spans bridged through
  `tracing-opentelemetry` in the *application's* graph — and `router()` is
  the API, so a service that wants request spans mounts it in its own axum
  app. TLS went the other way and is worth contrasting: it is *also* opt-in
  and off by default, so a deployment with a terminator in front installs a
  binary containing no TLS code — but a client certificate cannot be
  delegated to a terminator at all, because what a terminator passes on is a
  header, and a header is a claim.
- **Free-threaded CPython is declared, on narrower ground than the
  declaration sounds.** One interpreter (3.14t), one platform (manylinux
  `x86_64`/`aarch64`), ten repeated runs of the threading and shutdown
  suites: evidence, not proof. `cp313t` does not exist — PyO3 0.29 dropped
  it when CPython promoted free-threading to supported in 3.14.

**Deliberately still out**, each with a reason that is not "later": a
[`WriteDurability` mode](#writedurability-as-api-own) nobody has measured a
need for, the [runtime-agnostic S3 sleep](#runtime-agnostic-s3-watch-sleep-own)
that is blocked on the AWS SDK, [serde_yaml](#serde_yamls-future-own) which
moves when figment moves, [a ninth store](#a-store-nobody-has-asked-for-yet-own)
nobody has asked for, [msgspec as a fifth Python
schema](#msgspec-as-a-python-schema-own) waiting on somebody who actually
wants it, and [a book per crate](#one-book-or-a-book-per-crate-own) — where
the answer is probably per-crate entry *points* rather than fourteen books.

---

## Telemetry

### What a remote fetch does not yet report **[own]**

The reload path, the fetch path, the server's `/metrics` and the Python
binding all landed; [the book](book/src/telemetry.md) is the surface and the
changelog is the history. One piece is left, and it is now a wiring job
rather than a design question.

**A watch loop's failed attempts do not reach `RemoteStatus`.** `apply`
records a delivery, so a working watch keeps the status current — but a loop
whose stream broke, whose blocking query is erroring or whose credential was
refused delivers nothing, so `dynamic_config_remote_up` reports the last
*delivery* rather than the last *attempt*, and a store that stopped answering
an hour ago looks healthy until something calls `refresh_remote`.

The door exists: `RemoteSink::failed(&error)`, fenced on the sink's
generation exactly as `apply` is, moving only the failure streak and the last
failure so the staleness clock keeps running. What remains is one call at
each failure site in the seven network watch loops — Consul's retry branch,
etcd's stream-error and range-read branches, Redis' failed fetch and dead
subscription, NATS' stream error, and Vault, S3 and Firestore's poll
failures — reached through one `reporting_to(sink)` builder option per store
rather than a second `watch` method in seven crates. git needs none of it:
its watch is a poll, and a poll is a fetch, which already records itself.

---

## Correctness

An external review of the 0.5 branch produced four "P0"s. Each was checked
against the code before it was written down: two were real defects and one
was a contract that existed in a comment but not in the public
documentation — all three landed in 0.6. The fourth was a design decision
that had already been made, documented and tested, and it is kept here
because "asked and answered" is worth writing down once.

### `changes()` before `init()` — asked and answered **[own]**

The review asks for the first install *not* to wake a handle taken before
`init()`. That behaviour is deliberate, documented at `dynamic.rs:221`
("a handle taken before `init` sees the first install as its first
change — *wake me when configuration exists*"), and pinned by two tests
(`runtime_agnostic.rs:95`, `dynamic.rs:274`). It stays.

The alternative — first install is not a change — makes "wait until
configuration exists" unwritable without a second primitive, which is a
worse trade for the shape people actually have.

---

## Remote stores

### A store nobody has asked for yet **[own]**
`RemoteSource` and `AsyncRemoteSource` are public, so a new store is a crate
rather than a patch to this one. Seven exist; an eighth is worth adding when
somebody wants it, not before.

Each is a client dependency, a container in CI, an authentication story and a
set of failure modes to get right — the seven that exist took that seriously,
and an eighth done casually would be worse than none.

---

## Documentation

### One book, or a book per crate **[own]**

Sixteen crates share one mdBook, and the chapters that are *about a crate*
rather than about the engine are already the majority of it: eight store
pages, the config server and its threat model, the CLI, ten Python pages.
They are correct and they are in the wrong place — a reader who has added
`dynamic-config-vault` to a project does not want the engine's builder tour
first, and a store's own README is a paragraph pointing at a page in
somebody else's book.

**What a per-crate book would buy.** docs.rs already builds one thing per
crate; a book beside it would match how the crates are actually consumed
(one store at a time), let a store's chapter carry its own version, and stop
the root book growing a section per crate forever.

**What it would cost, and this is the decision.** Fourteen mdBook builds in
CI instead of one, fourteen link-check runs, and — the part that is not
mechanical — the cross-references. Half the value in the store chapters is
that they can say *this is the same `TlsConfig` every other store takes* and
link to it; split, that becomes an inter-book link that no tool checks and
that breaks silently when a page is renamed. The Python pages are worse:
they are the same engine described for another language, and half of what
they say is "as the Rust side does, here".

**The shape that is probably right** is neither: keep one book and make the
per-crate entry points real. A `book/src/crates/{name}.md` per crate, linked
from that crate's README as its front door, holding what is specific to it
and linking inward for what is shared — so a reader arriving from crates.io
lands on their crate and not on chapter one, without splitting a link graph
that is doing real work. It is worth doing when a store's chapter is long
enough that this is not just a redirect; today two of them are.

---

## The longer arc

### Instruction counts, not just wall clock **[own]**

Wall-clock benchmarks on a shared runner cannot gate a regression: the noise
is larger than the change worth catching. `iai-callgrind` counts instructions
under valgrind, which is deterministic enough to fail a pull request.

Not landed, and the reason is not "valgrind was missing" — it is that **the
baseline cannot be produced from a laptop**. iai-callgrind's whole value is
comparison against a committed baseline; it stores one under `target/`, which
does not survive between CI runs. Wired up without one, every run would be a
first run: it would print numbers and never fail. That is exactly the
benchmark that silently does nothing, and it would cost a lockfile entry and
a `cargo deny` review to have it.

What makes it landable is one maintainer action: a CI run on a branch that
installs valgrind, runs `cargo bench --bench instructions --save-baseline
main`, and commits what it produced.

### The road to 1.0 is stabilisation, not features **[own]**

Two releases in two days is a build phase, not a track record, and the
API surface is now wide enough that its cost compounds. Before 1.0: a
deliberate quiet period — 0.3 was the API-review release (the figment
leak pass; both of its fixes shipped in 0.4, and its keep-decisions live
in the stability-tiers chapter and the builder tour), 0.4 froze the shape
of the engine, 0.5 spent it on the bindings rather than widening the Rust
surface again, and 0.6 clears the backlog so that what remains is a
*decision* rather than a queue. Then: real external users, then a freeze
candidate.
New capability proposals queue behind stability during that window. The problem worth
solving by then is not a missing feature; it is that nothing this
sophisticated has been beaten up by strangers yet.

### msgspec as a Python schema **[own]**

The binding's schema surface is an adapter — `validate`, `field_names`,
`secret_paths`, `is_instance` — and there are four implementations of it
already: Pydantic, a Pydantic dataclass, a plain `dataclasses.dataclass`,
and `Values`, which is no schema at all.
[msgspec](https://github.com/jcrist/msgspec) is the obvious fifth: a
`msgspec.Struct` is a declaration in the same shape as the other two typed
ones, it validates on decode, and it is markedly faster than Pydantic at
exactly the thing this engine asks a schema to do — turn one resolved
mapping into one instance, once per reload.

**What makes it a decision rather than an afternoon.** The adapter's four
questions map cleanly (`msgspec.convert` for the validate half,
`msgspec.structs.fields` for the names), but two things do not:

- **Secrets have no declaration.** Pydantic has `SecretStr`, a dataclass has
  `field(metadata={"secret": True})`, and msgspec has neither — its
  `Meta`/`Annotated` carries constraints, not a place for a library's own
  flag. So either `Annotated[str, Meta(extra={"secret": True})]` becomes the
  spelling, which is this package inventing a convention in somebody else's
  namespace, or a msgspec configuration passes `secrets=[..]` the way a
  `Values` one does. The second is honest and already exists; it is also a
  second way to say a thing the other schemas say once.
- **The error shape.** `InvalidError.errors` carries Pydantic's own report,
  scrubbed of values, because a Python program branches on it. msgspec
  raises a `ValidationError` with a message and no structured report, so a
  msgspec configuration's `errors` would be empty — which is fine and has to
  be *said*, or it reads as a bug.

Neither is hard; both are decisions about a surface that is meant to look
the same whichever schema you brought. Worth doing when somebody is
actually reaching for msgspec — the extra is `dynamic-config-py[msgspec]`,
the adapter is one file next to `_pydantic.py`, and the base install goes on
depending on nothing.

### `WriteDurability` as API **[own]**
0.1.0 fsyncs every atomic write, unconditionally. If someone measures real
pain from that, a `Normal`/`Fsync` mode is the escape hatch — not before.

### serde_yaml's future **[own]**
Archived upstream (`0.9.34+deprecated`); figment pulls it regardless, so a
local switch buys nothing. Track figment; move when it moves.

### Runtime-agnostic S3 watch sleep **[own]**
Blocked on the AWS SDK itself being tokio-bound; revisit if smithy's
runtime abstraction ever makes executor-independence real.


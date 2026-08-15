# Roadmap

**This file is the engine's roadmap.** The stores, the server and the two
bindings plan in their own repositories — see
[the family](book/src/family.md). Everything below that mentions them is
history: it happened while they lived here.

What is not in the crate yet and might be. Everything that shipped is described
in [README.md](README.md); what will *not* be built, and why, is under
[Not planned](book/src/limitations.md#not-planned) there.

Tags: **[figment]** is something the underlying loader,
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

**0.6 shipped** — the whole family on crates.io, two wheels on PyPI, two
packages on npm, the book published, the instruction-count gate armed
with a committed baseline. It shipped from one repository; 0.6.1 is the
last release that did. What landed is not here: the changelogs carry it,
[README.md](README.md) describes the crate as it is, and this file keeps
only what is still open. Three of its items were answered by
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

**0.6 is complete, and so are the two remainders it left.** Instruction
counts: the workflow ran, the baseline is committed, and the gate now
fails a pull request that regresses past the limits
`benches/instructions.rs` declares. A remote fetch's reporting:
`RemoteSink::failed` and `Attempts` are wired through `reporting_to` in
all seven network stores, and git needs none of it because its watch is a
poll and a poll is a fetch.

**0.6.1 is the release being built now**, and it is the last one that adds
anything. Every crate and package moves to **Beta** with it: the store
crates' surfaces stopped moving two releases ago, each is tested against a
real server, each watch loop's failure branches are enumerated in its own
documentation, and three of them are unplugged mid-watch by `just chaos`.

**After it, only security fixes and hotfixes until 1.0.** No new sources,
no new stores, no new methods on the settled types. What still ships is a
defect that produces a wrong answer, a security advisory, and
documentation — each as a patch. Anything that would be nicer is a 1.0
candidate, written down here with its argument rather than slipped into a
0.x.
What it carries: the failure-branch *audit* behind the wiring above, with
chaos tests that unplug each store; a documentation and manifest sweep; a
Python book of its own; msgspec as a fifth Python schema; Node.js
bindings at 0.0.1, with a book of their own; and a third-party dependency
audit across all three ecosystems.

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
nobody has asked for. The book question is
[answered](#one-book-or-a-book-per-crate--answered-three-books-own): three
books, one per audience.

---

## Telemetry

### A watch that stopped delivering must not look healthy **[own]**

The reload path, the fetch path, the server's `/metrics` and the Python
binding all landed in 0.6; [the book](book/src/telemetry.md) is the surface
and the changelog is the history. So did the piece that was open when 0.6
was written: `RemoteSink::failed`, `Attempts`, and `reporting_to(sink)` on
all seven network stores. git carries none of it, deliberately — its watch
is a poll, and a poll is a fetch, which already records itself.

**The evidence landed in 0.6.1, and it changed two crates.** Every failure
branch of the seven loops is now marked in a table in its own crate's
documentation, under three rules that hold across all of them: a failure
the loop survives by retrying **reports**; a recovery that *worked* stays
**silent**, because only a delivery clears the streak and a five-minute
token turning over on a healthy cluster must not park `remote_up` at zero;
and a refusal that **never asked the store** reports nowhere.

The third rule is the one the audit settled rather than recorded. etcd and
NATS reported a watch refused before its first round trip — no format, a
key shape that cannot be watched — and Redis and S3 did not, each with a
test asserting its own half. `RemoteStatus::reachable()`'s contract
decides it: *whether the store answered the last time it was asked*. Those
refusals never ask, and a status carries a kind and a path and **no
message**, so `remote_up = 0` for a source typo is an alert about a
healthy store that nothing downstream can correct. etcd and NATS were
changed to match; the error still says exactly what is wrong, to the
caller holding it.

`just chaos` is the other half: toxiproxy in front of a store that never
restarts, so a cut cable and a restored one are both assertable. Three
loops are covered there, one per shape — Redis' subscription and etcd's
stream **end loudly**, Consul's blocking query **recovers on its own** —
and each asserts the pair an alert reads: `remote_up` goes to zero *while
the staleness clock keeps running*, and the last good document is still
being served. The three pollers (Vault, Firestore, S3) have the same
property proven without Docker, by their scripted-server suites: a poll
that fails reports, the loop survives, and the clock keeps ageing.

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

### One book, or a book per crate — **answered: three books** **[own]**

Three, one per audience, and the question is closed. `book/` is the Rust
book, `book-python/` and `book-node/` are the bindings', all three
published from one Pages deployment as `/dynamic-config/`,
`/dynamic-config/python/` and `/dynamic-config/node/`.

**A book per crate is not happening**, and the reason is the one the
argument kept returning to: half the value of a store chapter is that it
can say *this is the same `TlsConfig` every other store takes* and link to
it. Split into fourteen books, that becomes an inter-book link no tool
checks, and fourteen mdBook builds and fourteen link-check runs to
maintain it. The split that *was* worth making is by **language**, because
a Python or Node reader is not a Rust reader who wants a shorter chapter —
they are somebody who will never write `#[dynamic_config]`.

What remains available, and costs nothing to add later if a store chapter
ever grows long enough to need it, is a per-crate *entry point*: a page in
the Rust book that a crate's README links to as its front door. Today two
of the store chapters would be a redirect, which is why there is not one.

---

## The longer arc

### Instruction counts, not just wall clock **[own]** — shipped

Wall-clock benchmarks on a shared runner cannot gate a regression: the noise
is larger than the change worth catching. `iai-callgrind` counts instructions
under valgrind, which is deterministic enough to fail a pull request.

**Landed, and the shape is worth keeping** because the same trap waits for
any deterministic benchmark: iai-callgrind compares against a baseline it
stores under `target/`, which does not survive between CI runs — so wired up
without a *committed* one, every run is a first run that prints numbers and
can never fail. The gate is therefore two-state and says which state it is
in: no baseline means measure and warn, a baseline means compare and fail
past the limits `benches/instructions.rs` declares.

The baseline was produced the only way it can be: one CI run on the runner
image the comparison will use, its `iai-baseline` artefact committed under
`dynamic-config/benches/baselines/`. `current_once` is 85 instructions
there — the number the README's claim about a snapshot read is measured
against.

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

### `WriteDurability` as API **[own]**
0.1.0 fsyncs every atomic write, unconditionally. If someone measures real
pain from that, a `Normal`/`Fsync` mode is the escape hatch — not before.

### serde_yaml's future **[own]**
Archived upstream (`0.9.34+deprecated`); figment pulls it regardless, so a
local switch buys nothing. Track figment; move when it moves.

### Runtime-agnostic S3 watch sleep **[own]**
Blocked on the AWS SDK itself being tokio-bound; revisit if smithy's
runtime abstraction ever makes executor-independence real.


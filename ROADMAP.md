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
0.3 stabilised what existed (its list shipped whole; the story is the
changelog's), 0.4 adds the one engine piece everything after it needs,
and nothing jumps the queue without displacing something.

**0.4 — the instance engine, and the evidence:**

- `Dynamic<T>` + per-instance watch identity + value export — *shipped*:
  phase one of [python-bindings-plan.md](docs/python-bindings-plan.md), as
  Rust API on its own merits — [Dynamic Instances](book/src/dynamic-instances.md),
  `WatchKey`, `Snapshot::to_value`, its own test suite and bench row.
- Benchmarks that would convince a stranger — *shipped*: criterion over
  the read shapes, readers-during-reload, reload latency and load scaling
  to a hundred thousand keys, plus a counting-allocator profile that
  asserts the read path allocates nothing.
- A bundle for single-generation groups — *settled as a documented
  pattern*: one type, one section, one swap
  ([The Reload Lifecycle](book/src/reload-lifecycle.md)); a `Bundle`
  helper was considered and refused as a rename of "define one struct".
- `dynamic-config-cli` graduates to crates.io, redacted-by-default —
  *shipped*: `publish = false` dropped, README and LICENSE in place, the
  third publish wave and the dry-run lists know it, `explain` redacts
  unless `--show-values`, and `completions`/`man` render from the clap
  definition.
- Encrypting the last-known-good cache — *shipped*:
  `cache_encrypted(path, encryptor)`, full fidelity at rest, recovered
  through the installed `Decryptor`.
- Coverage threshold joins the gates — *shipped*: the coverage job fails
  under 80% lines (the number stabilised at ~87) and joined the required
  set; a floor against wholesale drops, not a target to chase.
- The figment review's two fixes — *shipped*: sections ride on a
  namespaced profile (`global`/`default` are ordinary section names, the
  silent override is gone), and environment provenance names the exact
  variable, derived from prefix + path + separator. The review document
  itself retired with its last open item — its keep-decisions live where
  they bind, in [Stability Tiers](book/src/stability-tiers.md) and the
  builder tour.
- The stability-tiers chapter says out loud that the `figment` feature is
  semver-coupled to figment — *shipped*.
- Housekeeping riding along: `scorecard.yml`'s trigger comment follows the
  default-branch flip (the fix sits on `dev`, with the note that a re-run
  of an old run replays its frozen event payload — dispatch fresh after a
  default-branch change).

**0.5 and later, pulled by demand:** the Python bindings proper (phases
two through five of the plan), `proc_macro_crate` rename, key aliases
across sections, `WriteDurability`, the embedded no-alloc wait queue, the
shared auth core and its dependents (`with_timeout` symmetry,
`ErrorKind::Auth`), runtime-agnostic S3 sleep, multi-key remote
documents, an eighth store when somebody asks, serde_yaml's future as
upstream decides it, and — far out, designed in the open first — the
config server.

## Layers

### Key aliases across sections **[viper]**
`alias("pool.size", "pool.max_size")` moves a path within one section. A value
that moved *between* sections — `server.timeout` becoming `http.timeout` — is
not expressible, because a `LoadSpec` resolves one section at a time.

Doable by resolving the other section during the alias pass. Unclaimed, and
worth a real case first: the cost is that a load then depends on a section the
type does not own.

---

## Writing

### Encrypting the last-known-good cache **[own]**
Shipped in 0.4: `cache_encrypted(path, encryptor)` — full fidelity at
rest, nothing readable on disk, recovery through the installed
`Decryptor`. The recipient-list objection that kept it out of the
attribute era dissolved with the builder: the recipients live in the
`Encryptor` the caller constructs, at the call site that owns them —
exactly the property `save_encrypted` was protecting.

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


### A bundle for single-generation groups **[own]**

Settled in 0.4, as a documented pattern rather than a helper: one type
holding both concerns, one section, one swap, one generation — written up
in [The Reload Lifecycle](book/src/reload-lifecycle.md) next to
`ReloadGroup`'s honest limit. A `Bundle` type was considered and refused:
it would be a rename of "define one struct", and machinery that restates
a design decision teaches people to skip the decision.

### Benchmarks that would convince a stranger **[own]**

Shipped in 0.4: `benches/engine.rs` (criterion — the three read shapes,
reads while a writer installs snapshots as fast as it can, reload
latency end to end, and pure loads at 10², 10⁴ and 10⁵ keys) and
`benches/alloc_profile.rs`, a counting allocator that *asserts* the
steady-state read path allocates nothing. The hand-rolled
`read_path.rs` stays as the loop the README quotes. Still open here:
iai-callgrind for instruction counts — it needs valgrind on the runner,
which is its own decision. Cross-library comparisons stay out of CI and
in a written-up experiment — they rot too fast to gate on.

### `dynamic-config-cli` on crates.io **[own]**

It ships in-repo, deliberately unpublished: crates.io versions are
permanent, and an Experimental surface should settle before it claims a
name. Next release it graduates: drop `publish = false`, give the crate its
own README and the symlinked LICENSE the packaging check demands, add it to
`release.yml`'s third wave and the dry-run's README/LICENSE list, and put
`cargo install dynamic-config-cli` in the book. Shell completions and a man
page ride along — clap generates both for one line each. Before it claims
the name, `explain` flips to redacted-by-default with `--show-values` to
opt in: an Experimental tool may ask the user to know which paths are
sensitive, a published one should not.

### A config server **[own]**

The other half of the distribution story, in the spirit of Spring Cloud Config Server:
a small service that owns the files (or fronts a store), serves resolved
sections over HTTP, and pushes changes to subscribed clients — so a fleet
of services shares one source of truth without each carrying store
credentials. The client side is already here (`RemoteSource` + a watch
loop); the server would be a new crate with its own threat model (authn,
who may read which section, audit). Far future, and worth designing in the
open before building.

### Python bindings: Rust resolves, Pydantic validates **[own]**

A PyO3 extension pairing this runtime with Pydantic: Rust owns sources,
layering, watching, recovery and provenance; Pydantic owns the schema and
its validators; Python reads a cached model for the price of an attribute
lookup, re-validated once per reload, never per read. Needs two core
changes that stand on their own — an instance engine (`Dynamic<T>`, for
every Rust user who wanted two configurations of one type) and a watch
identity beyond `TypeId`. The full design — decorator and class APIs, the
GIL strategy, secrets derived from `SecretStr` fields rather than
re-declared, the zero-bug test battery, wheels — is written up in
[python-bindings-plan.md](docs/python-bindings-plan.md), which is the
reference; this entry only tracks that it happens.

### The road to 1.0 is stabilisation, not features **[own]**

Two releases in two days is a build phase, not a track record, and the
API surface is now wide enough that its cost compounds. Before 1.0: a
deliberate quiet period — 0.3 was the API-review release (the figment
leak pass; both of its fixes shipped in 0.4, and its keep-decisions live
in the stability-tiers chapter and the builder tour); next, real external
users on 0.4+, then a freeze candidate. New capability
proposals queue behind stability during that window. The problem worth
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

### Coverage threshold + release gates **[own]**
Done in 0.4: the number stabilised (~87% lines), and the coverage job now
fails under 80 and sits in the required gate set — a floor against a suite
silently stopping, not a target to game.


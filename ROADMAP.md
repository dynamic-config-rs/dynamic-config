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

## The next two releases

Decided, not aspirational — each item below keeps its full description in
its own section; this is only who ships when. The shape follows
[the 1.0 doctrine](#the-road-to-10-is-stabilisation-not-features-own): 0.3
stabilises what exists, 0.4 adds the one engine piece everything after it
needs, and nothing jumps the queue without displacing something.

**0.3 — stabilisation.** No new surface except where an instruction
becomes a guarantee:

- Generation-fenced remote pushes — *shipped*: `remote_sink()` replaces
  `apply_remote`, and a replaced source's sink refuses by construction.
- loom model checking — *shipped*: `src/sync.rs` swaps the primitives
  under `--cfg loom`, and `just loom` proves the remote fence and the wake
  protocol over every interleaving. `ConfigCell`'s swap stays out — it
  lives inside `arc-swap`, which loom cannot instrument.
- Tidy the module tree — *shipped*: `builder/`, `watch/` and `redirects/`
  got the loader's treatment, one concern per file, and the onboarding
  tour maps the new layout.
- The figment abstraction-leak review from the 1.0 doctrine — *done*:
  [docs/figment-review.md](docs/figment-review.md). Two exposures stay
  deliberate, two changes land in 0.4 (the reserved-profile fix and
  derived environment provenance, below).
- Container suites that fail on behaviour, not scheduling luck.
- Security-tab triage — *done*: six open alerts triaged (four fixed by
  pinning patched versions, one blocked upstream and dismissed with a
  written reason, one already fixed), and the standing rule is in
  `SECURITY.md`.
- Release and branch mechanics, polished — *shipped*: the root changelog
  rotates from the pre-release hook, promotions squash-merge under a
  version-bearing title, and `main` becomes the default branch.
- Writing a store, promoted into the book — *shipped*:
  [Writing a Store](book/src/remote-stores/writing-a-store.md).

**0.4 — the instance engine, and the evidence:**

- `Dynamic<T>` + per-instance watch identity + value export — phase one
  of [python-bindings-plan.md](docs/python-bindings-plan.md), shipped as Rust
  API on its own merits.
- Benchmarks that would convince a stranger.
- A bundle for single-generation groups.
- `dynamic-config-cli` graduates to crates.io, redacted-by-default.
- Encrypting the last-known-good cache.
- Coverage threshold joins the gates.
- The figment review's two fixes
  ([docs/figment-review.md](docs/figment-review.md)): top-level tables
  named `global` or `default` stop colliding with figment's reserved
  profiles — today a `global` table silently overrides every section —
  and environment provenance names the exact variable, derived from
  prefix + section + separator + path, instead of `APP_*`.
- The stability-tiers chapter says out loud that the `figment` feature is
  semver-coupled to figment.

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
`save_encrypted` covers what a program deliberately writes. The
[cache](book/src/persistence.md#last-known-good) still writes plaintext, and its three modes
exist precisely because that is a trade-off.

A fourth mode — encrypted, full fidelity — would collapse the trade-off: it
would recover completely without leaving secrets readable. What stops it today
is that the cache is written on a path with no obvious place to put a recipient
list, and inventing a process-wide one would undo the reason `save_encrypted`
takes it at the call site.

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

## After 0.1.0


### A bundle for single-generation groups **[own]**

`ReloadGroup` promises all-or-nothing *installation* and says honestly that
the commits are separate swaps: a reader can still observe member A's new
generation next to member B's old one for an instant. For the few cases
where even that instant matters — a certificate and its port — the clean
answer is structural: one type holding both sections, one `ArcSwap`, one
generation. Worth offering as a documented pattern (or a small `Bundle`
helper) rather than leaving each caller to rediscover it.

### Benchmarks that would convince a stranger **[own]**

The read-path numbers come from a hand-rolled loop in this repository —
honest about what it is, but not evidence a skeptic can use. The upgrade:
criterion (and iai-callgrind for instruction counts), a concurrent
readers-during-reload scenario, reload latency, large-config scaling
(hundreds to hundreds of thousands of keys), and an allocation profile.
Cross-library comparisons stay out of CI and in a written-up experiment —
they rot too fast to gate on.

### Container suites that shrug off a slow daemon **[own]**

The store tests boot real servers, and a shared CI runner sometimes takes
longer to start one than the wait allows — `WaitContainer(StartupTimeout)`
from a Vault that was going to be fine in ten more seconds. That is a
false positive: the code did not change, the daemon was slow, and the fix
was "re-run the job". Built in instead, in 0.3: every suite's
`start_resilient` retries a failed startup once with a fresh container
before declaring failure, in both runner flavours — a second failure is
behaviour and reports both errors. Vault keeps its measured 120 s window
(it crossed 60 s once); the others stay on the default, because the retry
is what absorbs the tail now. Still standing as suites grow: keep the
`--test-threads=2` cap honest — the failure mode is concurrency-driven.

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

### Triage the Security tab, then keep it triaged **[own]**

GitHub's security surface (Dependabot alerts, code scanning, the
dependency-review scorecard warnings) accumulates findings that are each
either *fixable now*, *waiting on an upstream release*, or *consciously
accepted*. The first full pass happened in 0.3: `serde_with`, `actix-http`,
`quinn-proto` and `aws-sdk-s3` moved to their patched versions with
`cargo update --precise` (the MSRV-fallback resolver refuses the jumps on
its own — the `time` mechanism in `deny.toml`), and the `lru` soundness
advisory — held at 0.12 by `aws-sdk-s3`'s own requirement — was dismissed
with the reason on the alert. The standing rule now lives in
`SECURITY.md`: an alert is triaged within a release cycle or it blocks
one.

### Tidy the module tree **[own]**

Done in 0.3: `builder/` (fluent surface, lifecycle, diagnostics, watching,
the `Configured` slot), `watch/` (handle and registry, debounce loop,
relevance), `redirects/` (one macro family per file) — a directory, one
concern per file, the module doc naming what lives where, and the
onboarding tour updated to match. The crate split stays refused in
[Not planned](book/src/limitations.md#not-planned): folders and files, not
crates.

### A config server **[own]**

The other half of the distribution story, in the spirit of Spring Cloud Config Server:
a small service that owns the files (or fronts a store), serves resolved
sections over HTTP, and pushes changes to subscribed clients — so a fleet
of services shares one source of truth without each carrying store
credentials. The client side is already here (`RemoteSource` + a watch
loop); the server would be a new crate with its own threat model (authn,
who may read which section, audit). Far future, and worth designing in the
open before building.

### Writing a store, promoted into the book **[own]**

Done in 0.3: [Writing a Store](book/src/remote-stores/writing-a-store.md)
carries the watch loop and its `Watching` token, credential refresh with
one retry, the contract obligations and what the tests must pin — written
for a third-party author, with the workspace plumbing deliberately left to
the contributor guide.

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
deliberate quiet period — 0.3 as an API-review release (including a design
pass on where figment's abstractions leak into this crate's contract:
top-level tables, profile consumption, prefix-grained env provenance),
then real external users on 0.4+, then a freeze candidate. New capability
proposals queue behind stability during that window. The problem worth
solving by then is not a missing feature; it is that nothing this
sophisticated has been beaten up by strangers yet.

### Release and branch mechanics, polished **[own]**

A collection of paper cuts from cutting two releases, settled in 0.3:

- **`main` is the default branch** (a repository setting, flipped at the
  0.3 promotion). `scorecard.yml` moved its push trigger to `main` with it
  — the scorecard only publishes for the default branch.
- **Squash merges for `dev` to `main`.** `promote.sh` merges `--squash`;
  one commit per promotion, titled with the release, so `git log main`
  *is* the release history. `dev` is re-pointed at `main` afterwards, as
  before — the granular story is the changelog's to tell.
- **The promotion PR titles itself.** Both scripts compare the workspace
  version against `main`'s and title the PR "release X.Y.Z" when the push
  carries a bump (updating an already-open PR's title too); the squash
  reuses that title as the commit subject.
- **The root changelog rotates itself.** `scripts/rotate-root-changelog.sh`
  runs from the pre-release hook (idempotently — the hook fires once per
  package) and does the full hand-edit: dated heading, `[Unreleased]`
  compare link, and the released version's own reference link, which the
  per-package replacements in `release.toml` now also write for their own
  files.


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

### loom / shuttle model checking **[own]**
Landed in 0.3: the shims (`src/sync.rs`), the remote-fence models and the
wake-protocol model. What remains here is the harder residue — group
reload and the watch registry lean on process-wide statics, which loom's
iteration model does not tolerate, and `ConfigCell` sits behind
`arc-swap`. Shuttle, which runs real code unmodified, is the tool to
revisit for those.

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
Coverage reports exist (CI artifact + summary); a threshold waits until the
number stabilises post-0.1.0.


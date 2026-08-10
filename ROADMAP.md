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

## Sources

### A defaults struct **[figment]**
`set_default` takes one path at a time. figment's `Serialized::defaults(T)`
takes a whole struct, which is what most programs actually want — the defaults
written once, in the same shape as the configuration they back.

It works today through `Source::provider`. The open question is whether it
deserves an argument of its own, and what that would do to the precedence
diagram: defaults are the bottom layer, and a `Source` is not.

---

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

The 0.1.0 campaign (a three-way external review, fully implemented) closed
most of what used to live here: the remote race, watcher identity, panic
safety, debounce starvation, rename-aware secrets, recovery validation, the
big-file splits, the defaults struct, alias hardening, fsync, the book.
What remains below is what was *deliberately* deferred — each has a decision
in it, and is tracked as a GitHub issue.

### A real no-alloc wait queue for the embedded crate **[own]**
`ConfigCell<T, const WAITERS>` sizes the parking lot, but N > WAITERS still
degrades to wake-churn (documented). An intrusive list would fix it without
an allocator; it also drags `unsafe` into a crate that forbids it. That
trade deserves its own design pass.

### `WriteDurability` as API **[own]**
0.1.0 fsyncs every atomic write, unconditionally. If someone measures real
pain from that, a `Normal`/`Fsync` mode is the escape hatch — not before.

### loom / shuttle model checking **[own]**
The barrier tests pin the known interleavings; loom explores the unknown
ones. `Remote`'s state machine is the natural first target, but loom wants
its own sync-type shims — an investment, not an afternoon.

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

### Reading several keys as one document **[own]**
Unchanged from before: easy for etcd/Consul/Redis, awkward for
Vault/Firestore, needs a defined ordering. Possible, unclaimed.

### A store nobody has asked for yet **[own]**
Still true: an eighth store is worth adding when somebody wants it, not
before — and after the shared-auth-core question is settled.

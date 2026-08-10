# Roadmap

What is not in the crate yet and might be. Everything that shipped is described
in [README.md](README.md); what will *not* be built, and why, is under
[Not planned](README.md#not-planned) there.

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
[cache](README.md#last-known-good) still writes plaintext, and its three modes
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

## From the 0.0.1 review

A full pass over the workspace before the first release fixed what could be
fixed directly. These are the items that were *deliberately deferred* — each
has a decision in it, and the wrong week to make it is the one before a
release.

### Splitting the three big files **[own]**
`dynamic-config-macros/src/expand.rs` (~1,300 lines, one function near 900),
`dynamic-config/src/lib.rs` (a long wall of `#[doc(hidden)]` redirect macros),
and `dynamic-config/src/loader.rs` (providers, aliasing, sectioning in one
file). All three are readable top-to-bottom today, and that is worth something;
mechanical splits would churn every open branch and blur `git blame` right
before the history becomes public. The stacked-`#[cfg]` bug the size *did*
hide has been fixed directly. Split when a real change collides with the size,
not before — and when it happens, split by what a contributor searches for
(`expand_watch`, the redirect macros, the provider wall), not by line count.

### A shared auth core for the HTTP stores **[own]**
Consul, Vault and Firestore each carry their own `Session`/`Token` pair with
the same shape: cache a token, renew it near expiry, invalidate on a refused
request, log in at most once concurrently. Three copies is one too many — but
extracting a `dynamic-config-auth` crate means an eleventh crate, a public
contract for something currently private, and Vault's renewal semantics
(leases, renewable flags) do not quite fit Consul's (nanosecond TTLs) or
Firestore's (expiry only). Reconcile them on paper first; extract second.

### `with_timeout` for etcd, NATS and S3 **[own]**
The three ureq-based stores take `with_timeout`. The other three configure
timeouts through their client's own vocabulary (`ConnectOptions`,
`SdkConfig`), which is the documented pattern — but the asymmetry is real and
surprises people. Either add pass-through methods (more API to keep) or
document the asymmetry as a decision in each README (cheaper, done for etcd).
Decide once, for all three, when somebody actually trips on it.

### Coverage and semver gates in CI **[own]**
Two jobs that only earn their keep after the first release:
`cargo-llvm-cov` (with a threshold low enough not to fight refactoring) and
`cargo-semver-checks` (which needs a published baseline to compare against —
before 0.0.1 exists on crates.io there is nothing to check). Add both in the
first post-release change to CI.

### An `ErrorKind::Auth` variant **[own]**
The store crates now classify auth failures internally (typed 401/403
matching), but the public error is still `ErrorKind::Remote`. A caller who
wants "credentials are wrong, do not retry" as a program-visible state has to
parse the message. `ErrorKind` is `#[non_exhaustive]`, so the variant is
additive — the open question is the boundary: is a Vault 403 on a *path*
(policy) the same kind as a 403 on a *token* (expiry)? Decide with a real
consumer in hand.

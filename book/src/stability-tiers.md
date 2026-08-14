# Stability Tiers

Every crate in this workspace is **Beta**, and one is deliberately not a
tier at all.

| Crate | Tier |
|---|---|
| `dynamic-config` | **Beta** |
| `dynamic-config-macros` | **Beta** |
| `dynamic-config-etcd` | **Beta** |
| `dynamic-config-consul` | **Beta** |
| `dynamic-config-nats` | **Beta** |
| `dynamic-config-redis` | **Beta** |
| `dynamic-config-vault` | **Beta** |
| `dynamic-config-s3` | **Beta** |
| `dynamic-config-firestore` | **Beta** |
| `dynamic-config-git` | **Beta** |
| `dynamic-config-embedded` | **Beta** |
| `dynamic-config-server` | **Beta** |
| `dynamic-config-cli` | **Beta** |
| `dynamic-config-py`, `dynamic-config-py-remote` | **Beta** |
| `dynamic-config-node`, `dynamic-config-node-remote` | **Beta** |
| `dynamic-config-store-core` | no API — see below |

**The store crates were Experimental until 0.6.1, and what moved them is
evidence rather than time.** Each is tested against a real server in a
container; each watch loop's failure branches are enumerated in a table in
that crate's own documentation; three of them are unplugged mid-watch by
`just chaos` — toxiproxy in front of a store that never restarts — and
asserted to report the outage without losing the document they were
serving. The surfaces stopped moving two releases ago. That is what the
old tier said the path out was, and this is it.

## What Beta promises

The core crate and the macro are **Beta**. The API is settled enough to build
on, but pre-1.0 it may still break — and when it does, the break lands in a
minor version bump (`0.x` → `0.(x+1)`), is called out in the
[changelog](https://github.com/ctolon/dynamic-config/blob/main/CHANGELOG.md),
and comes with what to change on your side. A patch release never breaks. MSRV
is treated as a breaking change here too, so a toolchain bump follows the same
rule.

What the promise covers is what rustdoc renders. A `#[doc(hidden)]` item is
`pub` because something has to reach it across a crate boundary — the code the
attribute generates, or this repository's own fuzz targets — and it may be
renamed or removed in a patch release. `__private` and `__fuzz` are the two,
and neither appears in this book for the same reason it does not appear in the
API documentation.

## What happens between here and 1.0

**Only security fixes and hotfixes.** The surface is what it is going to
be for 0.x: no new sources, no new stores, no new methods on the settled
types. What still lands is a defect that produces a wrong answer, a
security advisory, and documentation — and each of those goes out as a
patch.

That is a change of intent, not of policy, and it is worth saying plainly
because the two read the same from outside: a project that publishes
weekly because it is growing and a project that publishes rarely because
it is finished both look quiet. This one is the second.

**What it means for a program that depends on this.** Pin the minor
version and take patches automatically; a patch will not break you, and
the release that could is the 1.0 that is being worked towards. An API
that would be nicer is a 1.0 candidate — written down in
[the roadmap](https://github.com/ctolon/dynamic-config/blob/main/ROADMAP.md)
with its argument — rather than something to slip into a 0.x.

## `dynamic-config-store-core` promises nothing

It is published because a crate that a published crate depends on has to be —
cargo resolves a path dependency from the registry when it packages. It holds
what the store crates share: the credential cache, URL redaction, the watch
panic net. Nothing outside this workspace should depend on it, and it carries
no compatibility promise at all, not even Experimental's.

## The `figment` feature is a coupling, on purpose

With the `figment` feature on, `Source::provider` and the `pub use
figment` re-export make figment's own API part of this crate's public
surface — so a figment major release forces either a major release here
or a compatibility shim, and 1.0 of this crate will not extend its
stability promise across that boundary. That is the feature's price and
its point: it exists precisely so the long tail of sources this crate
will never ship stays reachable without forking. With the feature off —
the default — a figment major bump is not a breaking change here.

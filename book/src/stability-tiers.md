# Stability Tiers

Not every crate in this workspace makes the same promise. Two tiers, and every
crate is in exactly one of them:

| Crate | Tier |
|---|---|
| `dynamic-config` | **Beta** |
| `dynamic-config-macros` | **Beta** |
| `dynamic-config-etcd` | Experimental |
| `dynamic-config-consul` | Experimental |
| `dynamic-config-nats` | Experimental |
| `dynamic-config-redis` | Experimental |
| `dynamic-config-vault` | Experimental |
| `dynamic-config-s3` | Experimental |
| `dynamic-config-firestore` | Experimental |
| `dynamic-config-git` | Experimental |
| `dynamic-config-embedded` | Experimental |
| `dynamic-config-server` | Experimental |
| `dynamic-config-store-core` | no API — see below |

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

## What Experimental promises

The eight store crates, the embedded crate and the server are
**Experimental**. They work, they are tested (the store crates against real
servers in containers, the git store against real repositories, the embedded
crate against a real `thumbv7em-none-eabihf` build, the server against its own
router), and the contracts described in this book hold — but their APIs may
change shape without ceremony: a release may rename, restructure or remove
things with no deprecation cycle, noted in the changelog but not negotiated
there. Pin an exact version if you depend on one. The path out of Experimental
is use: a crate whose surface has stopped moving gets promoted to Beta.

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

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
| `dynamic-config-embedded` | Experimental |

## What Beta promises

The core crate and the macro are **Beta**. The API is settled enough to build
on, but pre-1.0 it may still break — and when it does, the break lands in a
minor version bump (`0.x` → `0.(x+1)`), is called out in the
[changelog](https://github.com/ctolon/dynamic-config/blob/main/CHANGELOG.md),
and comes with what to change on your side. A patch release never breaks. MSRV
is treated as a breaking change here too, so a toolchain bump follows the same
rule.

## What Experimental promises

The seven store crates and the embedded crate are **Experimental**. They work,
they are tested (the store crates against real servers in containers, the
embedded crate against a real `thumbv7em-none-eabihf` build), and the contracts
described in this book hold — but their APIs may change shape without ceremony:
a release may rename, restructure or remove things with no deprecation cycle,
noted in the changelog but not negotiated there. Pin an exact version if you
depend on one. The path out of Experimental is use: a store crate whose surface
has stopped moving gets promoted to Beta.

# Comparisons

> What this engine is built on and whose ideas it took is
> [CREDITS.md](https://github.com/ctolon/dynamic-config/blob/main/CREDITS.md).
> This page is about the differences.

This page is for the decision *before* the one this book otherwise answers:
whether to use this crate at all. Three alternatives come up every time —
`serde` and a format crate on their own, [config-rs](https://docs.rs/config),
and [figment](https://docs.rs/figment) — and for a large share of programs one
of them is the better answer.

None of the four is a superset of the others, and the differences are not
mostly about quality. They are about *when* configuration is read. serde,
config-rs and figment all answer "what is the configuration?" once, at startup,
and hand back a value. This crate answers it continuously: it keeps the
snapshot, re-reads the sources when they change, and serves the current one to
every thread. Everything else here follows from that, on both sides of the
ledger.

## serde and a format crate

```rust
let text = std::fs::read_to_string("config.toml")?;
let config: Config = toml::from_str(&text)?;
```

For a program that reads one file at startup and never reloads, this is the
right answer, and adding a configuration library to it is a cost with no
return. The whole mechanism is two lines you can read; there is no precedence
to reason about, no global storage, and the configuration is an ordinary value
you pass to the code that needs it — which is easier to test than anything
below, because a test just constructs one.

It stops being enough at a specific point, and the point is worth naming rather
than assuming: when a second source has to override the first (an environment
variable beating a file, a flag beating both), when you have to answer *which*
source set a value, when the file has to be re-read while the process runs, or
when a value must be kept out of an error message. Until then, adding layers
buys nothing.

`envy` covers the neighbouring case — configuration entirely from environment
variables, deserialized into a struct — with the same shape and the same
absence of ceremony.

## config-rs

The most widely used configuration crate in the ecosystem, and the default
answer for good reasons. Sources are added to a builder — files, environment
with a prefix and a nesting separator, in-memory maps — and the result either
deserializes into a struct or is read a key at a time with `get::<T>("db.port")`.

**Where it is the better choice:**

- **Formats this crate does not read.** config-rs parses INI, RON and JSON5 as
  well as JSON, TOML and YAML. None of the first three is here, and
  [none is planned](limitations.md#not-planned) — each is a parser and a set of
  edge cases for a format nobody has asked for.
- **Configuration that is not one struct.** Reading `get::<u16>("db.port")`
  without declaring a type for the document is config-rs's ordinary mode. Here
  the typed struct is the centre of the design; the untyped road exists
  ([Schemaless Configuration](schemaless.md)) but it is the smaller half.
- **A custom source with no third-party type in your signature.** config-rs's
  `Source` trait is public and names only config-rs types. The equivalent here
  is [`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider),
  which takes a *figment* provider — a coupling that is
  [deliberate and opt-in](stability-tiers.md#the-figment-feature-is-a-coupling-on-purpose),
  but a coupling. If a parser plug-in is the requirement rather than a
  convenience, config-rs asks less of you.
- **Mileage.** It is older, more depended upon, and has been through more
  production configurations than this crate has.

**Where it stops:** a `Config` is a value, built once. Keeping it current is
yours to arrange — a file watcher, a rebuild, and an `RwLock<Config>` that
every read then goes through. That is a real amount of code, and the lock is on
the read path, which is the thing [`current()`](hot-reload.md#reading-is-lock-free)
exists to avoid. Errors name the source a bad value came from; there is no
per-key report of which layer won.

## figment

figment is what this crate is built on, so the comparison is unusually
concrete: **everything figment does is still reachable from here**, and
everything this crate reports about provenance *is* figment metadata,
converted at the one moment the figment that knew is still alive.

Providers are merged in order, profiles select between variants, `extract()`
produces the struct, and `Metadata` records where each value came from.
Alongside that it carries two things this crate does not reimplement: `Jail`,
a test helper that gives a test its own directory and environment, and magic
values like `RelativePathBuf`, which resolves against the file that named it
rather than against the working directory.

**Where it is the better choice:**

- **You are already holding one.** Rocket's configuration is a figment. Using
  the mechanism your framework already exposes beats adding a second one.
- **You want figment's profiles.** This crate spends profiles on **sections** —
  `builder("db")` selects the `db` profile — and rebuilds the profile *idea*
  on top with [`profile_env`](profiles-and-discovery.md#profile_env) and
  sibling files. A provider's own profiles
  therefore cannot pass through; see
  [Not planned](limitations.md#not-planned).
- **You want to assemble and extract yourself.** If the configuration is read
  once and stored wherever your application already stores things, the storage
  and reload machinery here is weight without a job.
- **Many instances of one configuration type.** A plain figment has no global
  anything. Here storage is keyed by the type, and several live instances of
  one type is a separate mechanism
  ([`Dynamic<T>`](dynamic-instances.md)) rather than the default.

**Where it stops:** the same place config-rs stops. A `Figment` is built and
extracted; re-reading it later, deciding whether the new document is even
valid, and keeping the old one serving if it is not, are all yours.

## What this crate adds

Three things, and they are the whole of the difference:

- **The snapshot and the read path.** `current()` is an atomic pointer load, so
  configuration can be read per request rather than threaded through
  constructors, and [a bad edit degrades to "no change"](hot-reload.md#reloading-cannot-take-the-process-down)
  instead of taking the process down.
- **Provenance as an API rather than as an error string.**
  [`source_of`, `explain`, `check` and key-level diffs](validation-diagnostics.md#where-did-this-value-come-from)
  answer "which file, variable or store set this key, and what did it beat?"
  for any key, at any time.
- **Secrets as a property.** Diagnostics, diffs, reports and `{:?}` print paths
  and types and never values, and a test suite fails the build when that stops
  being true. In the other three, keeping a resolved password out of a log is
  your discipline rather than the library's.

Around those sit the pieces that only make sense once configuration is live:
[remote stores](remote-stores.md) under one watch contract, the last-known-good
cache, encrypted files, schema export, and a CLI that explains a load without
running the program.

## What it costs

- **Weight.** figment, serde and arc-swap are unconditional; everything else is
  a [feature or a companion crate](features.md). serde and a format crate is a
  smaller dependency than any of the three libraries, and always will be.
- **Global storage.** A configuration type's snapshot, layers and bindings live
  in statics keyed by the type. That is what makes `current()` free from
  anywhere, and it is also why two tests sharing a config type share state —
  the [testing chapter](testing.md#pin-configuration-with-the-override-layer) is
  about working with that, not around it.
- **Maturity.** The engine is Beta pre-1.0: breaking changes bump the minor and
  are announced, but they happen. config-rs and figment are both older and more
  widely deployed.
- **A higher floor for the live half.** The core compiles on an older toolchain
  than the `watch` feature does; the table is in
  [MSRV & Features](msrv-features.md).
- **Constraints the design implies.** Every top-level key in a file must be a
  table, because sections are profiles, and a provider's own profiles cannot
  pass through. Both are in [Limitations](limitations.md).

## Choosing

| If | Then |
|---|---|
| one file, read at startup, never re-read | serde and a format crate |
| environment variables only, into a struct | `envy` |
| several sources, read at startup, never re-read | config-rs or figment |
| INI, RON or JSON5 | config-rs |
| a value read without declaring a type for the document | config-rs |
| a custom parser, with no third-party type in the signature | config-rs |
| you are already on Rocket, or already hold a figment | figment |
| figment's own profiles, or many instances of one type | figment |
| the file must be re-read while the process runs | this crate |
| reads happen per request and must not take a lock | this crate |
| "which source set this key?" must be answerable at runtime | this crate, or figment metadata by hand |
| a secret must never reach a log, a diff or a report | this crate |
| a remote store — etcd, Consul, Vault, S3 — as a layer | this crate |

The rows are not weighted, and the list is not a score. Most programs match the
first row.

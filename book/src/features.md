# Cargo Features

The core crate ships nearly everything behind a feature, so a build
carries only what it asked for: `dynamic-config` with default features
pulls in no cryptography, no HTTP client and no async runtime. This
chapter is what each flag actually buys — the surface it unlocks, the
dependency it pulls, and the reason it is a choice rather than a default.

```toml
[dependencies]
dynamic-config = { version = "<version>", features = ["toml", "watch", "async"] }
```

How a *missing* feature fails is part of the design. Surface whose
feature is off does not exist — `watch()` without `watch` is a compile
error, not a runtime surprise. A *format* whose feature is off fails at
load time instead, naming the feature to add, because the file's path is
runtime data now: the message says exactly what to put in
`features = [..]`, so the one machine that reads YAML diagnoses itself.

## Formats

| Feature | Default | Unlocks |
|---|---|---|
| `json` | ✅ | `.json` sources, via `serde_json` |
| `toml` | | `.toml` sources |
| `yaml` | | `.yaml` / `.yml` sources |

One per format because each is its own parser dependency. `json` is the
default as the least controversial single choice; turn it off with
`default-features = false` if the build reads only TOML. The extension
picks the parser at load time, which is why these are load-time rather
than compile-time failures when missing.

## Watching

**`watch`** — `builder.watch()` / `watch_with()`, the generated
`start_watch()`, and `Dynamic::watch`: the debounced, directory-level
file watcher. Pulls [`notify`](https://docs.rs/notify), the platform
notification backend — and raises the core MSRV to 1.85, which is
`notify 8`'s floor. The one feature with a real MSRV cost on the core.

## Async

**`async`** — `load_async` / `init_async`, `changes()`,
`Dynamic::changes`, and the `AsyncRemoteSource` trait the async store
crates implement. **No dependency at all**: `Changes` is a hand-written
`Future`, so tokio, smol, Embassy and a hand-rolled executor all drive
it; blocking work goes to a fresh thread unless an executor is installed
with `set_blocking_executor`.

**`tokio`** — `async`, plus tokio's blocking pool instead of a thread
per load. This is a routing choice, not a requirement: an application
already on tokio avoids a thread spawn per reload. Does not raise the
MSRV.

## Integration

**`clap`** — `bind_clap(&matches, ..)`: copies named command-line
arguments into the flags layer. The only feature that pins another
crate's *major* version, which is exactly why it is opt-in and separate —
with it off, a clap major release is not this crate's breaking change.

**`figment`** — `Source::provider(&dyn figment::Provider)` and figment
itself re-exported: the escape hatch for the long tail of sources this
crate will never ship (a database, an in-house format). The only feature
that puts figment in a public signature; with it off, a figment major
bump is not a breaking change here, and with it on you have opted into
that coupling knowingly. See
[Stability Tiers](stability-tiers.md).

**`dotenv`** — `.env_file(..)`: a `.env` file read as the environment
layer, below the real environment. Deliberately does not call `setenv` —
mutating the process environment to configure one struct is a side
effect nobody asked for, and it is not thread-safe.

## Schema

**`schema`** — `builder.schema()` and `schema::merge`: a JSON Schema for
the config *files*, secrets marked `writeOnly`, which is what gives
editors validation and completion. Pulls `schemars` (the type needs
`derive(JsonSchema)`) and raises the MSRV to 1.74, schemars' own floor.

## Encryption

**`decrypt`** — the `Decryptor`/`Encryptor` traits, `set_decryptor`, and
`encrypted_file(..)` on the builder: bring your own scheme (SOPS through
`sops -d`, a KMS). Pulls `zeroize`, so decrypted plaintext is wiped on
drop whatever the scheme is.

**`age`** — `decrypt`, plus the stock implementation: transparent
decryption (and `save_encrypted`) for `age`-encrypted files. MSRV 1.85 —
measured against a real toolchain, not `age`'s own claim; see
[MSRV & Features](msrv-features.md) for why the two differ.

## Observability

**`tracing`** — the watcher's diagnostics become `tracing` events
instead of stderr lines, every install becomes a `dynamic_config.reload`
span carrying the reason, the generation and the outcome, with a `WARN`
event for a reload that installed nothing, and every remote fetch becomes
a `dynamic_config.fetch` span carrying the outcome and, on a failure, the
error kind. Nothing is emitted on the read path, and no field carries a
value, a key path or a store's address. See [Telemetry](telemetry.md).

**`telemetry`** — `telemetry::Exposition`: `ConfigStatus` and
`RemoteStatus` rendered as Prometheus text, so a process can serve
`/metrics` without this crate choosing its metrics ecosystem. **No
dependency at all** — an exposition format is a wire encoding, not a
crate — and the metric names are API: six for the configuration, six more
for a remote source, each labelled by the caller rather than by anything
the crate read. Also see [Telemetry](telemetry.md).

## The bundle

**`full`** — everything above. For applications that want the whole
surface; a library depending on this crate should name what it needs
instead, so its users' builds stay small.

## What is *not* a feature

The remote stores: each is a [companion crate](remote-stores.md), not a
flag, so reaching for Consul does not put a gRPC stack, the AWS SDK and
three HTTP clients into a build that asked for none of them. The
`no_std` engine is likewise [its own crate](no-std-embedded.md) — a
microcontroller needs a different shape, not fewer flags. MSRV per
configuration is [its own chapter](msrv-features.md).

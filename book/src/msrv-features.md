# MSRV & Features

What each feature unlocks, and why it is a choice, is the
[Cargo Features](features.md) chapter's subject; this page is the cost
side — which flags move the compiler floor, and how the floors are kept
honest.

## Minimum supported Rust version

| Configuration | MSRV |
|---|---|
| any format, `tokio`, `tracing`, `telemetry`, `dotenv`, `figment` | 1.71 |
| `watch` enabled | 1.85 (`notify 8` requires it) |
| `schema` enabled | 1.74 (`schemars` requires it) |
| `dynamic-config-cli` | 1.85 |
| `dynamic-config-python` | 1.85 (PyO3), and CPython 3.9+ through abi3 — plus 3.14t, which is not abi3 (see below) |
| `age` enabled | 1.85 — measured, not declared (see below) |
| the companion crates (etcd, Consul, Vault, Firestore) | 1.85 |
| `dynamic-config-nats`, `-redis`, `-s3` | 1.88 (their clients require it) |
| `dynamic-config-git` | 1.85 (`gix` requires it) |
| `dynamic-config-server` | 1.80 (axum 0.8 requires it) |
| `dynamic-config-store-core` | 1.71 — it holds nothing but `std`, so it can never be what raises a store's floor |
| `dynamic-config-python-remote` | 1.88 — it carries all eight store clients, so it inherits the highest floor among them |
| `dynamic-config-embedded` | 1.83 (`core::error::Error` in `no_std` needs 1.81) |

Only `watch`, `schema` and `age` raise the floor for the core crate; `tokio`
does not.

`age` declares 1.74 for itself, and the figure above is 1.85 because that is
what actually builds: `age` pulls `rust-embed` for its translations, which pulls
`sha2 0.11`, which is edition 2024. The number here is the one the CI job
verifies against a real toolchain, not the one a manifest claims. A companion
pays for what it pulls in — a gRPC stack, a streaming client, an HTTP client —
and the core stays where it is.

The Python floor has a second dimension the others do not: the *interpreter*.
One abi3 wheel per platform covers CPython 3.9 upwards, which is why the wheel
carries no per-version matrix — but a free-threaded interpreter has no stable
ABI, so `dynamic-config-py` also ships a `cp314t` manylinux wheel, built
against that interpreter specifically. `abi3` is a default Cargo feature that
build switches **off**, because cargo features are additive and nothing can
turn abi3 off by being turned on; what actually selects the ABI is which
interpreter maturin is pointed at. 3.14t and not 3.13t: PyO3 0.29 dropped
3.13t when CPython promoted free-threading from experimental to supported in
3.14.
[Free-Threaded CPython](python/free-threading.md) is the audit behind the
declaration.

MSRV is treated as a breaking change, and both figures are verified in CI
against the real toolchains.

Contributors: this repository sets
`resolver.incompatible-rust-versions = "fallback"` in `.cargo/config.toml`.
Without it, cargo resolves to the newest release of every transitive dependency
and the floor silently becomes 1.85 — `hashbrown 0.17`, reached through `toml`,
requires edition 2024. Generating the lockfile needs cargo 1.84 or newer.

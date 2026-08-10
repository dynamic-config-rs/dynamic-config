# Releasing

Ten crates, versioned together, published in three waves:

```text
dynamic-config-macros          first, always
  └── dynamic-config           second
        ├── dynamic-config-etcd
        ├── dynamic-config-consul
        ├── dynamic-config-nats        third, in any order
        ├── dynamic-config-redis
        ├── dynamic-config-vault
        ├── dynamic-config-s3
        └── dynamic-config-firestore

dynamic-config-embedded        independent — published in the first wave
```

`dynamic-config` depends on `dynamic-config-macros` with an exact requirement
(`=x.y.z`), so a version mismatch is impossible — and so the macro crate must
always go first. The seven store crates depend on `dynamic-config` the same way,
which is why they come last. `dynamic-config-embedded` depends on neither, so
CI publishes it in the first wave alongside the macros.

## Releasing

`cargo release` prepares; CI publishes. The split is deliberate: a laptop cannot
push to crates.io without the checks having run.

```sh
cargo install cargo-release just

cargo release patch --execute     # 0.0.1 -> 0.0.2
cargo release minor --execute     # 0.0.1 -> 0.1.0
```

That runs `just check`, bumps every crate, moves each `## [Unreleased]` section
under a dated version heading, commits, tags `vX.Y.Z` and pushes. The tag is
what starts [`release.yml`](.github/workflows/release.yml), which verifies the
tag matches the manifest and the changelog names the version, then publishes in
three waves.

### Before you run it

1. `main` is green, including the container job and every MSRV row.
2. `CHANGELOG.md` — and each companion's — has entries under `Unreleased`. A
   release with an empty section is a release nobody can read.
3. The README's install snippet names the version you are about to publish.
   `cargo release` does not rewrite it, because it appears in prose as often as
   in a code block and a regex that catches all of them catches too much.

### If it has to be done by hand

The waves exist because each crate pins the one below it exactly, so a wave
cannot resolve until the previous one is on the registry:

```sh
cargo publish -p dynamic-config-macros
cargo publish -p dynamic-config-embedded    # depends on nothing here
# wait for the index, usually under a minute
cargo publish -p dynamic-config
# wait again
cargo publish -p dynamic-config-etcd
cargo publish -p dynamic-config-consul
cargo publish -p dynamic-config-nats
cargo publish -p dynamic-config-redis
cargo publish -p dynamic-config-vault
cargo publish -p dynamic-config-s3
cargo publish -p dynamic-config-firestore
```

`--no-verify` is deliberately not used. The verification build is the last
chance to catch a package that resolves locally through a path dependency and
nowhere else.

### Afterwards

Check docs.rs built each crate with `all-features = true`, so feature-gated
items carry their badges — and that each companion rendered *its own* README
rather than the workspace one.

## Version policy

- **Pre-1.0, a breaking change bumps the minor version** and everything else the
  patch. `0.0.x` is the pre-announcement series: the API is expected to move.
- MSRV changes are breaking. Every figure in the README's MSRV table is part of
  the public contract — including the companion crates' floors (1.85, or 1.88
  where the client requires it: NATS, Redis, S3), which are higher than the
  core's 1.71 on purpose: a companion pays for what it pulls in.
- figment is the loader, and its behaviour is this crate's behaviour. A figment
  upgrade that changes how values are merged or how environment strings are read
  is a breaking change here even when no signature moves — `tests/loader.rs`
  exists to make that visible rather than surprising.
- Four crates **do** re-export pieces of their client's API, so a major bump
  in the client is breaking for them: etcd and NATS re-export `ConnectOptions`
  and `Client`, Redis re-exports `redis::Client`, and S3 takes the SDK's
  `SdkConfig` and `Client` in its signatures. That is the price of not
  inventing a second vocabulary for credentials, and it is paid by those
  crates alone. Nothing from `ureq` or `base64` appears in the Consul, Vault
  or Firestore signatures.
- The traces a value carries are contract too: `Origin::Remote` names whatever
  the source's own `describe()` returns, so changing that string in a companion
  crate changes what users see in an error.
- `Error`, `ErrorKind` and `Origin` are `#[non_exhaustive]`, so new variants are
  additive.

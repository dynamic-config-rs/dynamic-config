# Everything CI runs, in the order that fails fastest.
#
# The container-backed suites are separate: they need a Docker daemon, and a
# contributor without one should still be able to run everything else.

default: check

# fmt, clippy, tests, docs — the whole gate, locally. No Docker needed:
# `test` excludes the container-backed crates, which live in `containers`.
check: fmt lint test docs embedded

# Formatting, as CI checks it.
fmt:
    cargo fmt --all -- --check

# Clippy with warnings denied, at both ends of the feature range.
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo clippy --workspace --all-targets --no-default-features -- -D warnings

# The whole suite, plus the two configurations that only exist with features
# off. The container crates are excluded — their tests drive real servers and
# belong to `containers`; their non-Docker mock tests still run there too.
test:
    cargo test --workspace --features full \
        --exclude dynamic-config-etcd --exclude dynamic-config-consul \
        --exclude dynamic-config-nats --exclude dynamic-config-vault \
        --exclude dynamic-config-redis --exclude dynamic-config-s3 \
        --exclude dynamic-config-firestore --exclude dynamic-config-embedded
    cargo test -p dynamic-config --no-default-features --lib --tests
    cargo test -p dynamic-config --no-default-features --features json --test ui
    cargo test -p dynamic-config --no-default-features --features async,json

# The scripted-server tests in the store crates: no Docker, fast, and they
# pin the retry decisions the container suites cannot see.
mocks:
    cargo test -p dynamic-config-consul --test mock_agent
    cargo test -p dynamic-config-vault --test mock_vault
    cargo test -p dynamic-config-firestore --test mock_firestore

# The loom models: every interleaving of the remote fence and the wake
# protocol, on the real code (`src/sync.rs` swaps the primitives). Only the
# `loom` test target builds under `--cfg loom` — the runtimes the examples
# use have their own loom wiring the flag alone does not satisfy.
loom:
    RUSTFLAGS="--cfg loom" cargo test -p dynamic-config --no-default-features --features json,async --test loom --release

# Every benchmark, the way CI runs them: the hand-rolled read path, the
# criterion suite (reads, readers-during-reload, reload latency, load
# scaling), and the allocation profile, which asserts the read path
# allocates nothing.
bench:
    cargo bench -p dynamic-config --features json -- --quick

# Documentation, with the badges docs.rs renders. Needs a nightly toolchain
# (`rustup toolchain install nightly`), the same way docs.rs builds it.
docs:
    RUSTDOCFLAGS="-D warnings --cfg docsrs" cargo +nightly doc --workspace --all-features --no-deps

# The companion crates, against real servers. Needs a Docker daemon.
containers:
    cargo test -p dynamic-config-etcd -p dynamic-config-consul \
               -p dynamic-config-nats -p dynamic-config-vault \
               -p dynamic-config-redis -p dynamic-config-s3 \
               -p dynamic-config-firestore

# The `no_std` crate, on a host and for a target with no `std` at all.
embedded:
    cargo test -p dynamic-config-embedded --features std,async
    cargo check -p dynamic-config-embedded --target thumbv7em-none-eabihf \
                --no-default-features --features json,async

# Every MSRV floor, against real toolchains rather than a manifest's word —
# the same rows as CI's msrv matrix. The committed lockfile is put back
# afterwards, whatever happens: a fresh `generate-lockfile` resolves with
# the MSRV fallback, which silently drops every security `--precise` pin —
# that is how four patched versions regressed at once, once.
msrv:
    #!/usr/bin/env bash
    set -euo pipefail
    cp Cargo.lock Cargo.lock.pinned
    trap 'mv Cargo.lock.pinned Cargo.lock' EXIT
    cargo +stable generate-lockfile
    cargo +1.71 check -p dynamic-config --locked --no-default-features --features json,toml,yaml
    cargo +1.71 check -p dynamic-config --locked --no-default-features --features json,async,tracing,clap
    cargo +1.71 check -p dynamic-config --locked --no-default-features --features json,dotenv,figment
    cargo +1.74 check -p dynamic-config --locked --no-default-features --features json,schema
    cargo +1.85 check -p dynamic-config --locked --no-default-features --features json,age
    cargo +1.85 check -p dynamic-config --locked --no-default-features --features full
    cargo +1.85 check -p dynamic-config-etcd --locked
    cargo +1.85 check -p dynamic-config-consul --locked
    cargo +1.88 check -p dynamic-config-nats --locked
    cargo +1.85 check -p dynamic-config-vault --locked
    cargo +1.88 check -p dynamic-config-redis --locked
    cargo +1.88 check -p dynamic-config-s3 --locked
    cargo +1.85 check -p dynamic-config-firestore --locked
    cargo +1.85 check -p dynamic-config-cli --locked
    cargo +1.83 check -p dynamic-config-embedded --locked --no-default-features --features json,async

# Every pairwise feature combination compiles — CI's `features` job.
# Needs cargo-hack (`cargo install cargo-hack`).
hack:
    cargo hack check -p dynamic-config --feature-powerset --depth 2
    cargo check -p dynamic-config --no-default-features --features figment
    cargo check -p dynamic-config --no-default-features --features decrypt

# The declared minimum dependency versions actually resolve — CI's
# `minimal-versions` job. Needs a nightly toolchain. The committed
# lockfile is put back afterwards — regenerating one is what loses the
# security pins, not what restores them.
minimal-versions:
    #!/usr/bin/env bash
    set -euo pipefail
    cp Cargo.lock Cargo.lock.pinned
    trap 'mv Cargo.lock.pinned Cargo.lock' EXIT
    cargo +nightly generate-lockfile -Z direct-minimal-versions
    cargo +stable check -p dynamic-config --locked --all-features

# Advisories, licences and registries.
audit:
    cargo deny check

# Every example, built the way CI builds them.
#
# The companion crates are built one at a time: their example names have to be
# unique across the workspace only because cargo puts them all in one output
# directory, and building them separately is the alternative cargo suggests.
examples:
    cargo build -p dynamic-config --features full --examples
    cargo build --examples -p dynamic-config-etcd -p dynamic-config-consul \
                -p dynamic-config-nats -p dynamic-config-vault \
                -p dynamic-config-redis -p dynamic-config-s3 \
                -p dynamic-config-firestore

# The book, the way CI builds it before publishing to Pages. Needs mdbook
# (`cargo install mdbook`).
book:
    mdbook build book

# Regenerate the compile-fail expectations after an intentional change.
bless:
    TRYBUILD=overwrite cargo test -p dynamic-config --features full --test ui
    TRYBUILD=overwrite cargo test -p dynamic-config --no-default-features --features json --test ui

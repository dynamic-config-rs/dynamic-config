# Everything CI runs, in the order that fails fastest.
#
# The container-backed suites are separate: they need a Docker daemon, and a
# contributor without one should still be able to run everything else.

default: check

# fmt, clippy, tests, docs — the whole gate, locally. Nothing here needs
# Docker or a venv: the crates that did left with the stores and the
# bindings, and each of those repositories runs its own.
check: fmt lint test docs embedded

# Formatting, as CI checks it.
fmt:
    cargo fmt --all -- --check

# Clippy with warnings denied, at both ends of the feature range.
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo clippy --workspace --all-targets --no-default-features -- -D warnings

# The whole suite, plus the configurations that only exist with features
# off — the `no_std` cell and the engine's own minimum.
test:
    cargo test --workspace --features full
    cargo test -p dynamic-config --no-default-features --lib --tests
    cargo test -p dynamic-config --no-default-features --features json --test ui
    cargo test -p dynamic-config --no-default-features --features async,json

instructions:
    cargo bench -p dynamic-config --features json,toml --bench instructions

# The same harness, compiled but not run — which is all the ordinary gate
# can do without valgrind, and enough to catch a bench that stopped
# building.
instructions-check:
    cargo bench -p dynamic-config --features json,toml --bench instructions --no-run

# The loom models: every interleaving of the remote fence and the wake
# protocol, on the real code (`src/sync.rs` swaps the primitives). Only the
# `loom` test target builds under `--cfg loom` — the runtimes the examples
# use have their own loom wiring the flag alone does not satisfy.
loom:
    RUSTFLAGS="--cfg loom" cargo test -p dynamic-config --no-default-features --features json,async --test loom --release

# The shuttle models: the residue loom cannot reach — `ConfigCell` behind
# arc-swap, the hook list, group reload, and a `static` cell awaited through
# `changes()`. Randomised scheduling, but from a *fixed* seed, so a run is
# reproducible and CI can gate on it. `SHUTTLE_SEED` and `SHUTTLE_ITERATIONS`
# override; `just shuttle-soak` is the search.
shuttle:
    RUSTFLAGS="--cfg shuttle" cargo test -p dynamic-config --no-default-features --features json,async --test shuttle --release

# The same models, searching instead of regressing: a drawn seed (printed, so
# a failure is replayable) and forty times the schedules. Minutes, not
# seconds — run it when changing anything in `cell.rs`, `group.rs` or
# `asynchronous.rs`, not on every commit.
shuttle-soak:
    SHUTTLE_SEED=random SHUTTLE_ITERATIONS=2000000 RUSTFLAGS="--cfg shuttle" \
        cargo test -p dynamic-config --no-default-features --features json,async \
        --test shuttle --release -- --nocapture

# The fuzz targets compile — exactly what CI gates on. The *running* is on a
# schedule (`.github/workflows/fuzz.yml`), because a fuzz run is unbounded
# and a crash it finds is rarely the fault of the change in front of it.
# Needs nightly and cargo-fuzz (`cargo install cargo-fuzz`); `fuzz/` is its
# own workspace, so none of this touches the crates' lockfile or MSRV.
fuzz-build:
    cd fuzz && cargo +nightly fuzz build

# Each fuzz target, bounded. `just fuzz 300` to go longer.
fuzz seconds="60":
    #!/usr/bin/env bash
    set -euo pipefail
    cd fuzz
    for target in units redaction value_paths dotenv sections; do
      printf '\n\033[1m── %s\033[0m\n' "$target"
      cargo +nightly fuzz run "$target" -- -max_total_time={{ seconds }} -print_final_stats=1
    done

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
    cargo +1.71 check -p dynamic-config --locked --no-default-features --features json,dotenv,figment,telemetry
    cargo +1.74 check -p dynamic-config --locked --no-default-features --features json,schema
    cargo +1.85 check -p dynamic-config --locked --no-default-features --features json,age
    cargo +1.85 check -p dynamic-config --locked --no-default-features --features full
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

# This repository's book. The docs site builds it alongside the other
# three and publishes all four together; this is the same build, alone.
# Needs mdbook (`cargo install mdbook`).
book:
    mdbook build book
    test -f book/book/index.html

# Regenerate the compile-fail expectations after an intentional change.
bless:
    TRYBUILD=overwrite cargo test -p dynamic-config --features full --test ui
    TRYBUILD=overwrite cargo test -p dynamic-config --no-default-features --features json --test ui

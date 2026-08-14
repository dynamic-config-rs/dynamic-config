# Everything CI runs, in the order that fails fastest.
#
# The container-backed suites are separate: they need a Docker daemon, and a
# contributor without one should still be able to run everything else.

# What cargo names a cdylib here. Node loads the addon under one name on
# every platform, so this is the only place the difference is spelled —
# and a contributor on macOS gets a build rather than a `cp` that cannot
# find a `.so`.
lib_prefix := if os() == "windows" { "" } else { "lib" }
lib_suffix := if os() == "macos" { ".dylib" } else if os() == "windows" { ".dll" } else { ".so" }

default: check

# fmt, clippy, tests, docs — the whole gate, locally. No Docker needed:
# `test` excludes the container-backed crates, which live in `containers`,
# and the Python extension, whose suite is `python` (it needs a venv).
check: fmt lint test docs embedded

# Formatting, as CI checks it.
fmt:
    cargo fmt --all -- --check

# Clippy with warnings denied, at both ends of the feature range.
lint:
    cargo clippy --workspace --all-targets --all-features \
        --exclude dynamic-config-python \
        --exclude dynamic-config-python-remote \
        --exclude dynamic-config-node --exclude dynamic-config-node-remote -- -D warnings
    cargo clippy --workspace --all-targets --no-default-features \
        --exclude dynamic-config-python \
        --exclude dynamic-config-python-remote \
        --exclude dynamic-config-node --exclude dynamic-config-node-remote -- -D warnings
    # Their own lines, lib only: an extension module links no libpython and
    # an addon links no Node, so neither has a test target to build — and
    # clippy over `--all-targets` would try. The stores addon is also the
    # one crate that needs `protoc`, which is why no workspace-wide run
    # reaches it.
    cargo clippy -p dynamic-config-python --lib -- -D warnings
    cargo clippy -p dynamic-config-python-remote --lib -- -D warnings
    cargo clippy -p dynamic-config-node --lib -- -D warnings
    cargo clippy -p dynamic-config-node-remote --lib -- -D warnings

# The whole suite, plus the two configurations that only exist with features
# off. The container crates are excluded — their tests drive real servers and
# belong to `containers`; their non-Docker mock tests still run there too.
test:
    cargo test --workspace --features full \
        --exclude dynamic-config-etcd --exclude dynamic-config-consul \
        --exclude dynamic-config-nats --exclude dynamic-config-vault \
        --exclude dynamic-config-redis --exclude dynamic-config-s3 \
        --exclude dynamic-config-firestore --exclude dynamic-config-embedded \
        --exclude dynamic-config-python \
        --exclude dynamic-config-python-remote \
        --exclude dynamic-config-node --exclude dynamic-config-node-remote
    # The server's TLS suite needs its own line: it has no `full` feature, so
    # the workspace run above builds it with defaults and never compiles the
    # handshake, the key-permission refusal or the mTLS tests.
    cargo test -p dynamic-config-server --all-features
    cargo test -p dynamic-config --no-default-features --lib --tests
    cargo test -p dynamic-config --no-default-features --features json --test ui
    cargo test -p dynamic-config --no-default-features --features async,json

# The scripted-server tests in the store crates: no Docker, fast, and they
# pin the retry decisions the container suites cannot see.
mocks:
    cargo test -p dynamic-config-consul --test mock_agent
    cargo test -p dynamic-config-vault --test mock_vault
    cargo test -p dynamic-config-firestore --test mock_firestore
    # The credential-redaction and auth-classification unit tests live in
    # each store's `src/lib.rs` and need no server — they belong in the
    # fast gate, not behind Docker.
    cargo test -p dynamic-config-etcd --lib
    cargo test -p dynamic-config-nats --lib
    cargo test -p dynamic-config-redis --lib
    cargo test -p dynamic-config-s3 --lib

# The Python bindings: build the extension into a virtualenv, then run
# the suite, the type checker and the linter against it. Needs a venv
# (`python -m venv .venv && . .venv/bin/activate && pip install -e
# 'dynamic-config-python[dev]'`, or the same list by hand: maturin
# pytest pytest-asyncio pydantic pydantic-settings msgspec mypy ruff).
# `CARGO_TARGET_DIR` per recipe, and it is not a nicety: both wheels' cdylib
# is `[lib] name = "_core"`, so they write the same `lib_core.so`. Sharing a
# target directory makes the second `maturin develop` report "Finished in
# 0.14s" and install the *first* wheel's extension into the second package —
# which fails as `no attribute 'EtcdStore'`, a long way from the cause.
python:
    cd dynamic-config-python && CARGO_TARGET_DIR=../target/python maturin develop
    cd dynamic-config-python && python -m pytest tests -q
    cd dynamic-config-python && mypy --strict python/dynamic_config/ tests/typing/
    cd dynamic-config-python && ruff check .
    cd dynamic-config-python && ruff format --check .
    cd dynamic-config-python && for example in examples/[0-9]*.py; do echo "→ $example"; python "$example" > /dev/null || exit 1; done

# The opt-in remote wheel: the same gate, pointed at the other directory.
# It is a second extension module rather than a feature of the first — a
# wheel is built per platform, so the store clients cannot ride in the
# install that reads one TOML file. Needs `just python` to have run: the
# tests import the base package.
python-remote:
    cd dynamic-config-python-remote && CARGO_TARGET_DIR=../target/python-remote maturin develop
    cd dynamic-config-python-remote && python -m pytest tests -q
    cd dynamic-config-python-remote && mypy --strict python/
    cd dynamic-config-python-remote && ruff check .
    cd dynamic-config-python-remote && ruff format --check .

# The same suite on a free-threaded interpreter. A separate recipe because
# it needs a separate venv: `Py_GIL_DISABLED` is not abi3, so the wheel is
# built with `--no-default-features` and is version-specific — it cannot
# share an install with the abi3 one. Point VENV at a 3.14t venv
# (`uv python install 3.14t && uv venv --python 3.14t /tmp/ft`).
#
# The ten repeats are the point: the races the audit is about are timing
# dependent, and one green run of them is an anecdote.
python-free-threaded VENV:
    {{VENV}}/bin/python -c "import sys, sysconfig; assert sysconfig.get_config_var('Py_GIL_DISABLED'), 'not a free-threaded interpreter'"
    cd dynamic-config-python && VIRTUAL_ENV={{VENV}} CARGO_TARGET_DIR=../target/python-free-threaded {{VENV}}/bin/maturin develop --no-default-features
    {{VENV}}/bin/python -c "import sys, dynamic_config; assert not sys._is_gil_enabled(), 'importing dynamic_config re-enabled the GIL'"
    cd dynamic-config-python && {{VENV}}/bin/python -m pytest tests -q
    cd dynamic-config-python && for i in $(seq 1 10); do echo "→ iteration $i"; {{VENV}}/bin/python -m pytest tests/test_threading.py tests/test_shutdown.py tests/test_free_threaded.py -q || exit 1; done

# What a Python read costs, next to the things it is claimed to cost
# like. Not a gate — a shared runner cannot tell an attribute lookup from
# an attribute lookup — but the numbers the book quotes come from here.
python-bench:
    cd dynamic-config-python && python benchmarks/read_path.py

# Instruction counts, which unlike wall clock are stable enough to gate on.
# Needs valgrind and `iai-callgrind-runner` at EXACTLY the version of the
# `iai-callgrind` dev-dependency — the runner refuses a mismatch rather
# than reporting wrong numbers. See CONTRIBUTING.md.
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

# The companion crates, against real servers. Needs a Docker daemon.
containers:
    cargo test -p dynamic-config-etcd -p dynamic-config-consul \
               -p dynamic-config-nats -p dynamic-config-vault \
               -p dynamic-config-redis -p dynamic-config-s3 \
               -p dynamic-config-firestore

# The Node bindings: build the addon, then run the suite and the examples
# against it. Needs Node 18 or newer; nothing else — the suite is
# `node --test` and the facade is JavaScript with a hand-written `.d.ts`,
# so `npm install` is not part of this.
#
# `CARGO_TARGET_DIR` is shared with the workspace here, unlike the two
# Python recipes: this crate's cdylib has a name of its own, so nothing
# collides. The extension comes from `lib_suffix` at the top of this file.
node:
    cargo build -p dynamic-config-node
    cp target/debug/{{ lib_prefix }}dynamic_config_node{{ lib_suffix }} dynamic-config-node/index.node
    cd dynamic-config-node && node --test tests/*.test.js
    # The types a caller sees, checked the way their CI checks them —
    # skipped with a word rather than failing when TypeScript is not
    # installed, because `npm install -D typescript` is a choice this
    # recipe should not make for a contributor who is fixing Rust.
    cd dynamic-config-node && if [ -x node_modules/.bin/tsc ]; then \
        node_modules/.bin/tsc -p tests/typing/tsconfig.json && \
        node_modules/.bin/tsc -p examples/tsconfig.json; \
      else \
        echo "skipping the type check: npm install -D typescript"; \
      fi
    # Every runnable example. Three want a framework and say so rather
    # than failing when it is absent, so this needs no `npm install` —
    # what it proves without one is that they still start and exit.
    cd dynamic-config-node && for example in examples/*.mjs; do \
        echo "→ $example"; node "$example" > /dev/null || exit 1; \
      done

# The eight stores for Node, as a second package. Needs Node 18 or newer
# and nothing else: the suite constructs every store, checks that no
# description carries a credential, and drives the failure a store that is
# not there produces. A document actually arriving is the store crates'
# container suites, which already run against real servers.
node-remote:
    cargo build -p dynamic-config-node-remote
    cp target/debug/{{ lib_prefix }}dynamic_config_node_remote{{ lib_suffix }} dynamic-config-node-remote/index.node
    # The base package, linked the way npm would install it.
    mkdir -p dynamic-config-node-remote/node_modules
    ln -sfn ../../dynamic-config-node dynamic-config-node-remote/node_modules/dynamic-config-node
    cd dynamic-config-node-remote && node --test tests/*.test.js

# Chaos: a store unplugged mid-watch, and put back.
#
# Needs Docker, and starts *two* containers per test — the store, and a
# toxiproxy in front of it. The proxy rather than a stopped container is the
# whole trick: a restarted container comes back on a different host port, so
# "it recovered" could never be asserted against a source pointing at the old
# one. Nothing here restarts.
#
# They are `#[ignore]`d, so `just containers` skips them and this is what runs
# them. Three loops, one per shape: Redis' subscription and etcd's stream end
# loudly, Consul's blocking query recovers on its own. The three pollers prove
# the same property in `just mocks`, with a scripted 500 and no Docker at all.
chaos:
    cargo test -p dynamic-config-redis --test chaos -- --ignored --nocapture
    cargo test -p dynamic-config-consul --test chaos -- --ignored --nocapture
    cargo test -p dynamic-config-etcd --test chaos -- --ignored --nocapture

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
    cargo +1.71 check -p dynamic-config --locked --no-default-features --features json,dotenv,figment,telemetry
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
    cargo +1.71 check -p dynamic-config-store-core --locked
    cargo +1.85 check -p dynamic-config-git --locked
    cargo +1.80 check -p dynamic-config-server --locked
    cargo +1.80 check -p dynamic-config-server --locked --all-features
    cargo +1.85 check -p dynamic-config-cli --locked
    cargo +1.85 check -p dynamic-config-python --locked
    cargo +1.88 check -p dynamic-config-python-remote --locked
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
                -p dynamic-config-firestore -p dynamic-config-git
    # The server's TLS example is behind `required-features`, so it is
    # reached by neither the line above nor the workspace test run.
    cargo build -p dynamic-config-server --all-features --examples

# The book, the way CI builds it before publishing to Pages. Needs mdbook
# (`cargo install mdbook`).
book:
    mdbook build book
    # The binding's own book, into the first one's output — the layout CI
    # publishes: `/dynamic-config/` and `/dynamic-config/python/`.
    mdbook build book-python --dest-dir book/book/python
    mdbook build book-node --dest-dir book/book/node

# Regenerate the compile-fail expectations after an intentional change.
bless:
    TRYBUILD=overwrite cargo test -p dynamic-config --features full --test ui
    TRYBUILD=overwrite cargo test -p dynamic-config --no-default-features --features json --test ui

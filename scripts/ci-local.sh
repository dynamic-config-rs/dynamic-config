#!/usr/bin/env bash
# The whole CI gate, locally, in the order that fails fastest.
#
#   ./scripts/ci-local.sh           everything, containers and MSRV included
#   ./scripts/ci-local.sh --quick   the fast subset — what to run before a push
#
# Mirrors ci.yml deliberately: a gate that passes here and fails there is a
# bug in one of the two, and worth a look either way.
set -euo pipefail
cd "$(dirname "$0")/.."

quick=false
[ "${1:-}" = "--quick" ] && quick=true

step() { printf '\n\033[1m── %s\033[0m\n' "$*"; }

step "fmt"
cargo fmt --all -- --check

step "clippy, both feature extremes"
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace --all-targets --no-default-features -- -D warnings

step "workflows lint clean"
if command -v actionlint >/dev/null; then
  actionlint .github/workflows/*.yml
else
  echo "actionlint not installed — CI still runs it (cargo install actionlint, or your package manager)"
fi

step "the workspace suite (container crates excluded)"
just test

step "the loom models, every interleaving"
just loom

step "the scripted-server mocks"
just mocks

step "embedded: host tests + a target with no std"
just embedded

step "every example builds"
just examples

step "advisories, licences, sources, bans"
cargo deny check

if $quick; then
  step "quick gate done — containers, MSRV, serial rerun and docs were skipped"
  exit 0
fi

step "docs, the way docs.rs builds them"
just docs

step "the suite again, serialised (shared-state races pass alone)"
cargo test --workspace --features full \
  --exclude dynamic-config-etcd --exclude dynamic-config-consul \
  --exclude dynamic-config-nats --exclude dynamic-config-vault \
  --exclude dynamic-config-redis --exclude dynamic-config-s3 \
  --exclude dynamic-config-firestore --exclude dynamic-config-embedded \
  -- --test-threads=1

step "every MSRV floor, against real toolchains"
just msrv

if docker info >/dev/null 2>&1; then
  step "the seven stores, against real servers"
  just containers
else
  step "SKIPPED: containers — no Docker daemon. CI will still run them."
fi

step "the whole gate is green"

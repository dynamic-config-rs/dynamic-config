#!/usr/bin/env bash
# Rewrites every README install snippet to the version being released —
# the root's and the nine companions' — from cargo-release's pre-release
# hook, so the number can never lag the release again.
#
# The shapes rewritten here are exactly the shapes the doc_surface gate
# parses (`the_readmes_agree_on_one_version`): a `dynamic-config…` or
# `version` assignment carrying a bare x.y.z string. Prose mentions of a
# version are deliberately left alone — the old objection to automating
# this was that a regex loose enough to catch prose catches too much, and
# the answer is to not catch prose. The book never carries the number at
# all; its snippets say `<version>`.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${NEW_VERSION:?NEW_VERSION is set by the cargo-release hook environment}"

# A dry run rewrites nothing, exactly like the changelog rotation.
if [ "${DRY_RUN:-false}" = "true" ]; then
  echo "dry run: would sync README install snippets to $version"
  exit 0
fi

python3 - "$version" <<'PY'
import pathlib
import re
import sys

version = sys.argv[1]

# `dynamic-config = { version = "0.3.0", … }`, `dynamic-config-etcd = "0.3.0"`
# and the bare `version = "0.3.0"` inside a dependency table — one line at a
# time, so nothing outside an assignment is ever touched.
pattern = re.compile(
    r'^(\s*(?:dynamic-config[a-z0-9-]*\s*=.*?|version\s*=\s*)")(\d+\.\d+\.\d+)(")',
)

# Every README that carries a snippet, by name — silent string replacement
# is the mistake AGENTS.md lists, and a hardcoded roster is what makes a
# README dropping out of the sync a loud failure instead of a shrug. The
# CLI's README is exempt by design: a binary is installed, not depended on.
# A new companion crate joins this list, or the release fails saying so.
# 0.6 added two: `dynamic-config-git`, and `dynamic-config-server`, whose
# README carries a dependency snippet because its router is mountable in a
# service that already runs axum.
expected = 12

matched = 0
readmes = [pathlib.Path("README.md"), *pathlib.Path(".").glob("dynamic-config-*/README.md")]

for readme in readmes:
    lines = readme.read_text().splitlines(keepends=True)
    rewritten = []
    file_matches = 0

    for line in lines:
        new_line, hits = pattern.subn(lambda m: f"{m.group(1)}{version}{m.group(3)}", line)
        file_matches += hits
        rewritten.append(new_line)

    if file_matches:
        readme.write_text("".join(rewritten))
        matched += 1

if matched != expected:
    sys.exit(
        f"synced {matched} README(s), expected {expected} — a snippet was "
        f"deleted or rewritten into a shape this parser no longer sees"
    )

print(f"synced {matched} README(s) to {version}")
PY

#!/usr/bin/env bash
# Regenerates the dependency-weight table in book/src/msrv-features.md:
# crates-in-tree and clean-check seconds per representative feature set.
set -euo pipefail
cd "$(dirname "$0")/.."

sets=(
  "--no-default-features --features json"
  "--features json,toml,yaml"
  "--features full"
)

rows=""
for flags in "${sets[@]}"; do
  # shellcheck disable=SC2086
  crates=$(cargo tree -p dynamic-config $flags --edges normal --prefix none 2>/dev/null | sort -u | wc -l)
  cargo clean -p dynamic-config >/dev/null 2>&1
  start=$(date +%s)
  # shellcheck disable=SC2086
  cargo check -p dynamic-config $flags >/dev/null 2>&1
  seconds=$(( $(date +%s) - start ))
  label=${flags#--features }
  label=${label#--no-default-features --features }
  [[ "$flags" == --no-default-features* ]] && label="--no-default-features --features ${flags##* }"
  rows+="| \`${label}\` | ${crates} | ${seconds} |\n"
done

python3 - "$rows" <<'PY'
import pathlib, re, sys
rows = sys.argv[1].replace("\\n", "\n").rstrip()
p = pathlib.Path("book/src/msrv-features.md")
s = p.read_text()
s = re.sub(
    r"\| feature set \| crates in tree \| clean check \(s\) \|\n\|---\|---\|---\|\n(?:\|[^\n]*\n)*",
    "| feature set | crates in tree | clean check (s) |\n|---|---|---|\n" + rows + "\n",
    s,
)
p.write_text(s)
print("table regenerated")
PY

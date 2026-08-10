#!/usr/bin/env bash
# dev → main, the whole choreography:
#
#   push dev → ensure the pull request exists → wait for the gates →
#   merge (rebase, so the history stays linear) → re-sync dev onto main
#
# Safe to re-run at any point; each step is a no-op when already done.
# `main` takes no direct pushes — this is the only road, on purpose.
set -euo pipefail
cd "$(dirname "$0")/.."

current=$(git rev-parse --abbrev-ref HEAD)
if [ "$current" != "dev" ]; then
  echo "promote runs from 'dev' (you are on '$current')"
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "the working tree is not clean — commit or stash before promoting:"
  git status --short
  exit 1
fi

echo "── pushing dev"
git push -u origin dev

echo "── ensuring the pull request exists"
pr=$(gh pr list --base main --head dev --state open --json number -q '.[0].number')
if [ -z "$pr" ]; then
  gh pr create --base main --head dev \
    --title "promote dev to main" \
    --body "Promotes \`dev\` to \`main\`. Gates decide; this description does not."
  pr=$(gh pr list --base main --head dev --state open --json number -q '.[0].number')
fi
echo "pull request #$pr"

echo "── waiting for the gates"
# --fail-fast so a red gate stops the wait instead of running out the clock.
if ! gh pr checks "$pr" --watch --fail-fast; then
  echo
  echo "a gate is red — the merge is off. See: gh pr checks $pr"
  exit 1
fi

echo "── merging (rebase, linear history)"
gh pr merge "$pr" --rebase

echo "── re-syncing dev onto the new main"
# A rebase-merge gives the commits new SHAs on main, so dev is re-pointed at
# main rather than dragging duplicate history around. --force-with-lease so a
# push that arrived on dev meanwhile is a stop, not a casualty.
git fetch origin
git reset --hard origin/main
git push --force-with-lease origin dev

echo "── promoted. main is at $(git rev-parse --short origin/main)."
echo "if this bumped the workspace version, the merge just started a release:"
echo "  ./scripts/watch-release.sh"

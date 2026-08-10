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

echo "── arming auto-merge and waiting"
# Auto-merge instead of watching checks ourselves: the same commit can carry
# check runs from a cancelled twin (the push-run the PR-run deduplicated),
# and only GitHub's own merge logic knows which one counts. Auto-merge fires
# exactly when branch protection is satisfied — required gates green,
# conversations resolved.
gh pr merge "$pr" --rebase --auto

deadline=$((SECONDS + 3600))
while [ "$(gh pr view "$pr" --json state -q .state)" = "OPEN" ]; do
  if [ "$SECONDS" -ge "$deadline" ]; then
    echo "not merged within an hour — see what is holding it: gh pr view $pr"
    echo "(a red gate, or an unresolved conversation; auto-merge stays armed)"
    exit 1
  fi
  sleep 30
done

if [ "$(gh pr view "$pr" --json state -q .state)" != "MERGED" ]; then
  echo "the pull request closed without merging — investigate: gh pr view $pr"
  exit 1
fi

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

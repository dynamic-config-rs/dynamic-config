#!/usr/bin/env bash
# The first half of promote.sh, deliberately without the merge:
#
#   push dev → ensure the pull request exists → stop
#
# For when something should read the pull request before the gates decide —
# an `@claude review` comment, a colleague, your own second look. Nothing is
# armed: the PR sits open until you either merge it yourself or run
# ./scripts/promote.sh, which picks up from exactly here (both scripts are
# no-ops for what is already done).
set -euo pipefail
cd "$(dirname "$0")/.."

current=$(git rev-parse --abbrev-ref HEAD)
if [ "$current" != "dev" ]; then
  echo "propose runs from 'dev' (you are on '$current')"
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "the working tree is not clean — commit or stash before proposing:"
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

echo "── pull request #$pr is open, nothing armed"
gh pr view "$pr" --json url -q .url
echo
echo "ask for a review on it:   gh pr comment $pr --body '@claude review this'"
echo "merge when satisfied:     ./scripts/promote.sh"

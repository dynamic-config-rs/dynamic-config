#!/usr/bin/env bash
# Tags a release on main and watches the pipeline it starts.
#
#   ./scripts/tag-release.sh v0.0.1
#
# The tag is the release: release.yml verifies, publishes to crates.io in
# waves, and cuts the GitHub release. This script only makes sure the tag it
# pushes is one that workflow will accept — the same checks, run where a
# mistake is still free.
set -euo pipefail
cd "$(dirname "$0")/.."

tag="${1:?usage: tag-release.sh vX.Y.Z}"
case "$tag" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "'$tag' does not look like vX.Y.Z"; exit 1 ;;
esac
version="${tag#v}"

echo "── the tag goes on main, at origin's tip"
git fetch origin
declared=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
if [ "$version" != "$declared" ]; then
  echo "tag $tag does not match the workspace version $declared — release.yml would refuse it too"
  exit 1
fi

if ! grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no section for $version — release notes come from it"
  exit 1
fi

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists locally — a crates.io version is permanent, so nothing is re-tagged"
  exit 1
fi

echo "── tagging origin/main"
git tag -a "$tag" -m "dynamic-config $version" origin/main
git push origin "$tag"

echo "── the tag is pushed; watching the release run"
# The run takes a moment to appear after the tag lands.
for _ in 1 2 3 4 5 6; do
  run_id=$(gh run list --workflow Release --limit 1 --json databaseId,headBranch \
    -q ".[] | select(.headBranch == \"$tag\") | .databaseId" | head -1)
  [ -n "$run_id" ] && break
  sleep 5
done

if [ -z "${run_id:-}" ]; then
  echo "no release run visible yet — watch it at: gh run list --workflow Release"
  exit 0
fi

gh run watch "$run_id" --exit-status && echo "── released: $tag is publishing/published."

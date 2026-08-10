# Scripts

The `justfile` runs *checks*; these run *flows* — the git/gh choreography
around the checks. Each one is safe to re-run and says what it did.

| Script | What it does |
|---|---|
| `ci-local.sh` | The whole CI gate locally, in the order that fails fastest. `--quick` skips the slow suites (containers, MSRV). |
| `watch-ci.sh` | Watches the newest CI run for the current branch; on failure, prints the failed jobs' logs. |
| `promote.sh` | `dev` → `main`: pushes, opens the pull request if it is not already open, waits for the gates, merges (rebase), and re-syncs `dev` onto the new `main`. |
| `tag-release.sh vX.Y.Z` | On `main`: verifies the tag matches the workspace version and the changelog names it, tags, pushes the tag, and watches the release workflow it starts. |

The release itself is: land everything on `dev` → `./scripts/promote.sh` →
`./scripts/tag-release.sh vX.Y.Z`. Nothing here talks to crates.io; the tag
does, through CI, with the checks in front of it.

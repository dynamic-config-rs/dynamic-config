# Scripts

The `justfile` runs *checks*; these run *flows* — the git/gh choreography
around the checks. Each one is safe to re-run and says what it did.

| Script | What it does |
|---|---|
| `ci-local.sh` | The whole CI gate locally, in the order that fails fastest. `--quick` skips the slow suites (containers, MSRV). |
| `watch-ci.sh` | Watches the newest CI run for the current branch; on failure, prints the failed jobs' logs. |
| `promote.sh` | `dev` → `main`: pushes, opens the pull request if it is not already open, waits for the gates, merges (rebase), and re-syncs `dev` onto the new `main`. |
| `watch-release.sh` | Watches the Release run the latest merge to `main` set off, and says how to recover from a crates.io rate limit. |

The release itself is a pull request: run `cargo release patch --execute` on
a branch (it bumps, rewrites changelogs and commits — nothing else), land it
on `dev`, and `./scripts/promote.sh`. The merge to `main` is what publishes;
CI mints the tag and the GitHub release afterwards. Nothing here talks to
crates.io directly.

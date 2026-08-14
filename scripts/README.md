# Scripts

The `justfile` runs *checks*; these run *flows* — the git/gh choreography
around the checks. Each one is safe to re-run and says what it did.

| Script | What it does |
|---|---|
| `ci-local.sh` | The whole CI gate locally, in the order that fails fastest. `--quick` skips the slow suites (containers, MSRV). |
| `watch-ci.sh` | Watches the newest CI run for the current branch; on failure, prints the failed jobs' logs. |
| `claude-review-pr.sh` | Reviews a pull request with Claude locally — title, body, diff, and read-only access to the checkout. `--post` comments the review on the PR; without it, nothing leaves the terminal. |
| `propose.sh` | The first half of `promote.sh`: pushes `dev` and opens the pull request, then stops — for when something (an `@claude review`, a second look) should read the PR before anything merges. |
| `promote.sh` | `dev` → `main`: pushes, opens the pull request if it is not already open (titled "release X.Y.Z" when the push carries a version bump), arms auto-merge, waits for the gates, merges (squash — one commit per promotion, under that title), and re-syncs `dev` onto the new `main`. Picks up wherever `propose.sh` stopped. |
| `promotion-title.sh` | Sourced by the two scripts above — the one copy of the rule that titles a promotion ("release X.Y.Z" when the push carries a bump). Not run by hand. |
| `sync-readme-versions.sh` | Rewrites every README's install snippet to the version being released — assignment shapes only, never prose. Called by the pre-release hook; idempotent. Not run by hand. |
| `rotate-root-changelog.sh` | Rotates the workspace `CHANGELOG.md` for a release — dated heading, compare link, the version's own reference link. Called by cargo-release's pre-release hook; idempotent, so the per-package hook repetition is harmless. Not run by hand. |
| `watch-release.sh` | Watches the Release run the latest merge to `main` set off, and says how to recover from a crates.io rate limit. |
| `dismiss-alert.sh` | Dismisses a Dependabot alert with the reason recorded *on the alert* — GitHub's UI leaves that nowhere a reviewer finds later. `--list` shows what is open; the dismissal is confirmed before it is sent, and reopens by itself if the dependency graph changes. |
| `release-python.sh` | The Python packages' release, which is not the workspace's: bumps **both** wheels' versions, rotates both changelogs and moves the `dynamic-config-py>=…` floor between them, in one commit. `--check` refuses a version already on PyPI — the mistake that made the 0.6 wheel wave a silent no-op; `--publish` dispatches the wheel wave once the bump landed on `main`; `--status` says what is where, PyPI included. Prepares only — publishing stays CI's. |
| `resolve-python-audit.py` | What the wheels' extras actually resolve to today, as a requirements file. Neither package commits a lockfile — a library that pinned its dependencies would pin its users' — so the advisory scan and the SBOM both resolve it here rather than reading a range and guessing. |
| `audit-report.py` | An OSV scan's findings, split into *a fix exists* (fails the job) and *no fix published* (a warning and a line in the summary). Exemptions are `osv-scanner.toml`'s and never reach this. |
| `security-status.sh` | The whole security surface, read-only: open Dependabot alerts (with who pulls each package), open code-scanning findings, and cargo-deny's local view. Exits with the open-alert count. The triage rules it answers to are in `SECURITY.md`. |

The release itself is a pull request: run `cargo release patch --execute` on
a branch (it bumps, rewrites changelogs and commits — nothing else), land it
on `dev`, and `./scripts/promote.sh`. The merge to `main` is what publishes;
CI mints the tag and the GitHub release afterwards. Nothing here talks to
crates.io directly.

The two Python packages are the one exception, and `release-python.sh` is
why they need one: both are excluded from `cargo release` because the
wheels embed the engine rather than depending on a published version of
it, so their version moves when *they* change. They move **together** —
`dynamic-config-py[remote]` resolves to a pair, and the remote wheel
imports from the base one. Same shape as above — prepare, land, and let CI
publish — with `--publish` dispatching the wheel wave instead of a merge
triggering it.

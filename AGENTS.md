# AGENTS.md

Instructions for coding agents working in this repository. Humans want
[CONTRIBUTING.md](CONTRIBUTING.md); this file is the same ground rules with the
things an agent gets wrong made explicit.

## Orientation

Every README's install snippet names the version being cut — the root's and
the eleven companions' alike. The pre-release hook rewrites them all
(`scripts/sync-readme-versions.sh`), and `doc_surface.rs`'s
`the_readmes_agree_on_one_version` fails the gate if one is ever left
behind anyway. The book never carries the number at all —
its snippets say `<version>`.

Four crates in one workspace, one version, published together to
crates.io:

```text
dynamic-config-macros      the proc macro; no stable API of its own
dynamic-config             everything with behaviour — loading, layers, storage, watching
dynamic-config-embedded    a separate `no_std` crate, sharing no code
dynamic-config-cli         the `explain`/`diff` binary
```

The rest of the family lives in its own repository, each naming this
engine with a caret so a patch release here needs no release there:

```text
dynamic-config-remote      the eight stores, what they share, and the server
                           github.com/dynamic-config-rs/dynamic-config-remote
dynamic-config-python      two wheels on PyPI
                           github.com/dynamic-config-rs/dynamic-config-python
dynamic-config-node        two packages on npm
                           github.com/dynamic-config-rs/dynamic-config-node
```

A change to this crate's public surface is a change three repositories may
have to follow, and nothing in this build says so — which is what
`.claude/hooks/binding-drift.sh` prints while the change is still in hand.

`fuzz/` is its own workspace, so its lockfile and its nightly requirement
touch none of the above.

Read [README.md](README.md) before changing anything: it is the specification,
not a summary. [Not planned](book/src/limitations.md#not-planned) lists what is deliberately
*not* here and why; [ROADMAP.md](ROADMAP.md) lists what might still be. Check
both before building something that was already decided.

## Commands

```sh
just check        # fmt, clippy at both extremes, tests, docs, the no_std build
just embedded     # the no_std crate, on a host and for thumbv7em-none-eabihf
just msrv         # every MSRV floor, against real toolchains
just hack         # every pairwise feature combination compiles
just bless        # regenerate compile-fail expectations after an intended change
just book         # this repository's book
```

Nothing here needs Docker or a venv. The suites that did left with the
stores and the bindings, and each of those repositories runs its own.

There are skills in `.claude/skills/` for the tasks that recur:
[adding a Builder option](.claude/skills/add-macro-argument/SKILL.md),
[adding a Cargo feature](.claude/skills/add-cargo-feature/SKILL.md),
[triaging the security tab](.claude/skills/triage-security/SKILL.md), and
[reviewing before a release](.claude/skills/review-for-release/SKILL.md). Read
the relevant one before starting — each records decisions that are settled, so
you do not spend the turn re-deriving them.

`.claude/hooks/binding-drift.sh` runs after every edit and names the files a
change has to travel to. Here that is mostly *other repositories*: this
crate's public surface is what the stores implement against and what both
bindings wrap, and nothing in this build says when one of them goes stale.

Never claim a change works without running `just check`.

## Rules that are not negotiable

**Reading configuration is lock-free and allocation-free.** `current()`
acquires an `arc-swap` guard: **85 instructions** and zero allocations,
measured by `benches/instructions.rs` and `benches/alloc_profile.rs` rather
than asserted. Anything that puts a mutex, an allocation or a parse on that
path is wrong regardless of how convenient it is — and "an atomic load" is
the shape of the claim, not its cost.

**Secrets are paths and types, never values.** Diffs, `check()` reports,
unknown-key suggestions and *error messages* all report which key moved and what
type was expected — never what was there. `dynamic-config/tests/security.rs`
enforces this. A change that puts a value into a diagnostic is a security
regression even if every test still passes.

**The resolution is this crate's own**, and only the fold is swappable: an
`Engine` merges the collected layers and nothing else, every engine
implements the same merge rule, and the tests compare them leaf by leaf.
figment is one engine and one interop adapter — out of the default
dependency graph, in a public signature only behind the `figment` feature. That feature exists precisely so the coupling is opt-in; do not widen
it. figment stays a permanent dev-dependency, because the ported reader,
deserializer, serializer and fold are each proved against it — do not remove
those differential tests.

**`dynamic-config-embedded` shares no code with the rest**, and that is
deliberate: the core allocates a value tree and reads files, so there is
nothing to share. Do not try to unify
them. It keeps the *shape* — a snapshot in a `static`, a bad document leaving
the previous one serving, `changes()` — and nothing else.

**No mandatory dependency** beyond `serde`, `arc-swap` and the default
engine's crate; `--no-default-features` leaves the first two. Everything
else is a feature or a companion crate.

**`#![forbid(unsafe_code)]`** in every crate, checked by CI.

**Tests run on Linux, macOS and Windows, and a test may not assume which.**
The 0.6 release lost five CI rounds to this, each a different shape of the
same mistake, so the shapes are worth naming:

- **Never assert on how a path is *spelled*.** `with_file_name` rebuilds a
  path with the platform's separator, so `/etc/app/config.toml` becomes
  `/etc/app\config..toml` on Windows. Compare `Path` components — parent,
  extension, file name — not substrings or separator counts.
- **Never embed a path in generated TOML or JSON.** A Windows path in a TOML
  *basic* string makes `\a` an escape sequence and the file will not parse.
  Write forward slashes, which cargo and this crate's loader both accept
  everywhere.
- **Never let a `#[cfg(unix)]` block strand something outside it.** A `let mut`
  the block mutates, an import only it uses, a struct only its test builds —
  each is an error on Windows under `-D warnings`, and none is visible from
  the Unix branch. Prefer two whole functions over one with a block inside.
- **Do not put a watched file in the system temporary directory.** On macOS
  `/var` is a symlink to `/private/var`, so FSEvents reports a path the
  watcher was never registered on; on Windows the runner's `TEMP` is an 8.3
  short name and the events carry the long one. The engine's own watcher
  tests use `tests/scratch/` under the crate, and that is why.

`cargo check --tests --target x86_64-pc-windows-msvc` catches the
compile-time half from a Linux machine — for `dynamic-config` at least; the
crates that pull `ring` or `aws-lc-sys` need a Windows C toolchain and cannot
be cross-checked. The runtime half only the CI matrix finds.

**MSRV is measured, not declared.** The core floor is 1.71. A feature that
raises it says so in the README table *and* gets a row in the CI matrix — `age`
declares 1.74 and actually needs 1.85, which is the kind of thing only a real
toolchain finds.

## Mistakes this repository has actually seen

These are not hypothetical. Each one shipped, got caught, and cost a debugging
session:

**Tests that share state.** A config type's snapshot, layers, aliases and
bindings live in `static`s keyed by the type. Two tests using the same config
type, the same fixture path or the same environment variable will race — and
pass alone, which is worse. **One type, one fixture, one variable per test.**
Use a `macro_rules!` to declare them if that gets repetitive.

**Silent string replacement.** When editing files programmatically, assert the
anchor exists. A `replace` that matches nothing looks exactly like a successful
edit until something further downstream fails for an unrelated-looking reason.

**Believing a manifest.** `age` says 1.74 and needs 1.85. etcd's client claims
to connect and connects lazily. Measure, then write the number down.

**Cleanup that destroys the thing being protected.** `save_new` deleted the file
it had just refused to overwrite. Before removing anything on an error path,
ask whether this call is what created it.

**Assuming an executor's ordering.** Two tasks spawned together are polled in
whatever order the executor likes. Yield explicitly instead.

**Turning default features off without reading what they were.** An SDK's
defaults often include its HTTP client; removing them produces "no HTTP client
was available" at runtime rather than a compile error.

**Trusting a container registry.** A Docker Hub 429 looks exactly like a broken
test. Pre-pull in CI, and prefer a registry without anonymous limits.

**Deriving `Debug` over anything that can hold a credential or a fetched
document.** A derive prints every field; three store crates shipped 0.0.1
printing Vault/Consul/GCP tokens on `{:?}`. Hand-write `Debug` for any type
whose fields can carry a secret (redact the secret, keep the fields a
debugger needs), and add a planted-token test asserting `{:?}` excludes it.

**Stacking `#[cfg]` attributes.** Two `#[cfg]`s on one item AND together:
`#[cfg(unix)] #[cfg(not(unix))]` is unsatisfiable and compiles to *nothing*,
silently. Three tests in `write.rs` never ran for months because of exactly
that pair. One `cfg` per item; combine conditions with `all()`/`any()`.

**Emitting `#[cfg(feature = ...)]` from the proc macro.** A `cfg` in generated
code is evaluated against the *user's* crate features, where the feature does
not exist — the gated method silently vanishes for every user. Route it
through a `#[macro_export] #[doc(hidden)]` redirect macro defined in the
facade crate, where the `cfg` means what it says (see
`__clap_methods!` and `__async_methods!` in
`dynamic-config/src/redirects.rs`, and the add-cargo-feature skill).

## What a change must carry

- **A test that would fail without it** — not one that merely exercises the code.
- **The reasoning, where it is not obvious.** Comments here explain *why*; the
  code says what. If you chose between two reasonable designs, the rejected one
  belongs in a comment or in the roadmap.
- **Documentation** if a user would notice: a new `Builder` option goes in the
  book's attribute reference (`book/src/attribute-reference.md`, the Builder
  tables) and gets a section in the chapter it belongs to — the attribute
  itself takes no arguments, so there is no argument table to extend; a new
  feature goes in the feature tables (lib.rs front page and the book), and in
  the MSRV table if it moves the floor. A new generated method that skips the
  book fails `tests/doc_surface.rs`.
- **A `CHANGELOG.md` entry** under `Unreleased` — the workspace one, and the
  companion crate's own if that is what changed.

### If the change touches the public surface

Three repositories mirror this crate and nothing here notices when one goes
stale: the stores implement `RemoteSource` against it, and both bindings
wrap it. A signature change, a renamed variant, a new error kind — each is
a pull request in
[dynamic-config-remote](https://github.com/dynamic-config-rs/dynamic-config-remote),
[dynamic-config-python](https://github.com/dynamic-config-rs/dynamic-config-python)
and [dynamic-config-node](https://github.com/dynamic-config-rs/dynamic-config-node)
waiting to happen. They name this crate with a caret, so they pick a patch
release up on their own; what they cannot pick up on their own is a
*breaking* one.

## Where things live

| Looking for | Go to |
|---|---|
| what the crate does, and why each decision was made | `book/src/` — the book is the specification; `README.md` is the storefront |
| the stores, the server, and the bindings | their own repositories, and their own books — [the family](book/src/family.md) is the map |
| what is deliberately absent, and what would reopen it | `book/src/limitations.md` |
| what might still be built | `ROADMAP.md` |
| how a contributor gets started, and what every module does | `docs/CONTRIBUTOR-ONBOARDING.md` |
| the properties that must hold, and what enforces them | `SECURITY.md` |
| loading, merging, precedence | `dynamic-config/src/loader/` |
| what the attribute expands to | `dynamic-config-macros/src/expand/` |
| storage and reload hooks | `dynamic-config/src/cell.rs` |
| what the site publishes, and how | [dynamic-config-rs.github.io](https://github.com/dynamic-config-rs/dynamic-config-rs.github.io) — four books, one deployment |

## Style

`rustfmt` decides layout and `clippy -D warnings` decides the rest, at both
feature extremes. Beyond that: name things after what they mean to a caller.
Comments carry decisions, not mechanics — `// increments the counter` above
`counter += 1` is noise; `// bumped before the wake, so a waiter that polls
immediately sees the new generation` is not.

Prose in documentation is for a reader who is deciding whether to trust the
crate. State what it does *and* what it deliberately does not.

## Releasing

Do not publish. `cargo release` prepares and CI publishes on the tag; see
[RELEASING.md](RELEASING.md). Never run `cargo publish` directly.

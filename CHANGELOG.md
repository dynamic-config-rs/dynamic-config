# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Before 1.0, a breaking change bumps the **minor** version and anything else
bumps the patch. A change to the minimum supported Rust version is breaking.

<!-- Keep this template. Add entries under `Unreleased` as you go, and move
     the whole block under a new version heading at release time.
     (Spelled `_Unreleased_` here so cargo-release's `exactly = 1` search
     for the real heading matches only the real heading.)

## [_Unreleased_]

### Added
### Changed
### Deprecated
### Removed
### Fixed
### Security

-->

## [Unreleased]

## [0.6.1] — 2026-08-14

### Added

- **Node.js bindings**, as `dynamic-config-node` on npm: the engine, the
  watcher, the runtime layers, remote sources written in JavaScript and
  the whole diagnostic surface, through Node-API. A validator is a
  function, so Zod, Ajv or a plain function of your own all work and none
  of them is a dependency; `DynamicConfig<T>` is generic over whatever the
  validator returns, so `current()` is `T` under `strict: true`.

  The load runs on a worker thread and reaches the event loop only to
  validate and to fire hooks — which is what keeps the property this whole
  design is for: a document the schema refuses installs nothing and leaves
  the previous one serving, from the watcher exactly as from an explicit
  reload. It is also why there is no `initSync`: a synchronous load would
  be the loop waiting for itself.

  Twelve examples, a book of its own at `/dynamic-config/node/`, a
  `tsc --strict` gate over the definitions a caller sees, and a CI matrix
  across Node 18, 20, 22 and 24 — Node-API is ABI-stable, but the
  JavaScript half is ordinary code that a version can break. The release
  workflow gained an npm wave: five native runners, one prebuilt binary
  each, published as optional dependencies with provenance.
- **`dynamic-config-node-remote`**: the eight Rust stores for Node — etcd,
  Consul, Vault, NATS, Redis, S3, Firestore and git — as a second package,
  for the reason they are a second wheel in Python. Each is a class with
  an async `fetch()` and a `describe()`, which is the shape the base
  package's `setRemote` already took, so the two meet through a documented
  surface rather than through each other's internals. `useStore` is the
  bridge: a round trip must not sit on the event loop, and the remote
  layer is filled from a worker thread and must be handed a synchronous
  answer. Credentials may be **functions** that mint a fresh token per
  fetch, TLS material may be files *or* bytes, and the four stores that
  push — Consul, Redis, etcd, NATS — can be watched.
- **The Node binding caught up with the Python one**: `changes()` as an
  async iterator, `replace(document)` for a document this library did not
  fetch, `setDefaults(values)` for a whole object at once, and
  `strictEnv()`. What is deliberately still absent is `initSync`, and the
  book says why where a reader meets it.
- **A version table per binding**, the shape the Rust book's MSRV table
  is: which Python lines and which Node lines are supported, which are
  tested, and that raising either floor is a breaking change.
- **A `Patterns & Style` chapter in all three books** — one configuration
  per subsystem, read `current()` where you use it, hooks wake things
  rather than doing the work, what belongs in CI and what at startup, and
  the naming the files and variables use.
- **`Values.sub(path)`** in the Python binding: the subtree at a path, as
  a `Values` of its own, so a subsystem can be handed a section without
  being told where in the document it sits. `Snapshot::sub` is the Rust
  equivalent.
- **Every crate and package is Beta**, and the store crates' promotion is
  evidence rather than time: each is tested against a real server, each
  watch loop's failure branches are enumerated in its own documentation,
  and three are unplugged mid-watch by `just chaos`. **After 0.6.1, only
  security fixes and hotfixes until 1.0** — the three books say so where a
  reader will meet it.

### Changed

- **A watch refused before its first round trip no longer reports the
  store as unreachable**, in `dynamic-config-etcd` and
  `dynamic-config-nats`. `RemoteStatus::reachable()` is *whether the store
  answered the last time it was asked*, and a source with no format or an
  unwatchable key shape never asks — so `Some(false)` there was a status
  saying something untrue about a store that may be perfectly healthy.
  `dynamic-config-redis` and `dynamic-config-s3` already behaved this way;
  0.6.1's audit of all seven watch loops settled the split. Each crate's
  changelog carries its half, and every failure branch is now a table in
  its own crate's documentation.

  `just chaos` is the evidence: toxiproxy in front of a store that never
  restarts, three loop shapes, and the pair an alert reads — `remote_up`
  goes to zero *while the staleness clock keeps running*.

### Fixed

- **Four counts in the prose that the workspace had outgrown**, and a test
  that fails on the fifth: the ROADMAP said sixteen crates were on crates.io
  when fourteen publish, two pages said seven store crates when git made it
  eight, and `lib.rs`'s precedence chain had been missing `secrets_dir` since
  it landed — so the crate's front page described a layer order the loader
  did not have. `doc_surface` now counts the crates, compares the two copies
  of the precedence chain character for character, and checks that every
  example has a row in the book's table. The last one found `ini_provider`,
  which had compiled and run in CI for two releases with no way to find it.

### Added

- **Advisories are scanned across every ecosystem, not just Cargo.** The
  wheels ship Pydantic, pydantic-settings and msgspec as runtime
  dependencies of a published artefact, and `cargo-deny` has never heard of
  any of them. An `osv-scanner` job now reads the Cargo lockfile *and* what
  the wheels' extras resolve to today, against a database that aggregates
  RustSec, PyPA and GitHub advisories — so it is independent of
  `cargo-deny` rather than a second copy of it, and the two disagreeing is
  information.

  It fails on an advisory **with a fix available** and warns on one
  without: a repository that goes red for somebody else's release schedule
  is a gate people learn to ignore. `osv-scanner.toml` carries the narrow
  third case — a fix that exists and is pinned out of reach upstream — each
  entry with the reason and what would expire it.
- **Every GitHub release carries an SBOM per ecosystem**, CycloneDX JSON:
  one document per crate and one for the wheels' resolved dependencies.
  Attached, not gating.

### Changed

- **The Python binding has a book of its own**, at
  [`/dynamic-config/python/`](https://dynamic-config-rs.github.io/python/).
  Eleven chapters describing an engine through another language were a third
  of the Rust book's sidebar, and a Python reader arriving from PyPI landed
  in a table of contents whose first twenty entries were Rust. The store
  crates deliberately stay: a Consul chapter is read by whoever read the
  builder tour.

  **Published URLs did not move** — those chapters were already served from
  `/dynamic-config/python/…` — and `/dynamic-config/python.html` is now a
  stub that links onward, so nothing that pointed at either breaks. One Pages
  deployment, two directories; the link checker takes both books' sources.

### Changed

- **Every crate's crates.io metadata says what that crate is.** The stores
  inherited `categories = ["config", "development-tools"]` and one set of
  five keywords from the workspace, so a search for `etcd` or `vault`
  matched none of them and a client for a key/value store was filed under
  development tools. Each crate now carries its own: the stores are
  `api-bindings`, the server is `web-programming::http-server`, the macro
  crate is `development-tools::procedural-macro-helpers`, the CLI is
  `command-line-utilities`, and the keywords name the store rather than
  repeating the engine's. `dynamic-config-cli` also gained the
  `documentation` link it was inheriting from another crate.
- **Both wheels declare the interpreters they are tested against.** PyPI
  filters on the per-version classifiers, and `requires-python` alone does
  not populate them.

## [0.6.0] — 2026-08-13

### Added

- **A schemaless configuration: read by path, with no struct and no
  derive.** `Value` implements `Deserialize`, which is the only bound the
  engine puts on a configuration type, so `Builder::values(key)` and
  `Dynamic<Value>` are a configuration whose keys are learned at runtime —
  a plugin host, a feature-flag table, a tool inspecting somebody else's
  configuration. Layers, profiles, discovery, the watcher, the
  last-known-good cache, reload hooks, `source_of` and `explain` all work
  unchanged, because nothing in the engine ever knew what `T` was.

  **The cost is published rather than described.** One run of
  `benches/read_path.rs` on an i7-14700F: a struct field read 19.8 ns, a
  one-segment path 27.2 ns, a two-segment path 32.1 ns, `get_as::<u16>`
  36.7 ns — and zero allocations per 100 000 reads for every one of those
  except the typed read that hands back an owned `String`. **No feature
  flag and no new dependency**, deliberately: gating a trait impl would
  make `Dynamic<Value>` compile in one build and not another, and
  `DashMap` was rejected because reads are lock-free already — the
  snapshot is immutable and swapped whole.

  **A configuration with no schema is told what it is missing.** `Report`
  gained `unknown_checked`, so a check with no field list renders
  `unknown keys: not checked (no field list)` rather than an empty list
  that reads as an all-clear; `Builder::secrets(&[..])` supplies by hand
  the redaction list `#[config(secret)]` declares for a struct, and
  without one a redaction-dependent cache mode is still refused at `init`.

- **Reading several keys as one document, in all eight store crates.** A
  deployment that splits its configuration across a prefix (`myapp/db`,
  `myapp/server`) installed one source per section; every store now reads
  the set and hands the loader one document. `Keys::several([..])` merges
  named keys **in call order — later wins**, exactly the way `.file(..)`
  layering already works, because a caller who wrote the list wrote the
  precedence with it. `Keys::prefix("myapp/")` **refuses an overlap**,
  naming both keys and the paths they collided on — a caller who wrote a
  prefix wrote no order, and the order a server lists keys in is nobody's
  decision. Every constructor takes `impl Into<Keys>` and a bare `&str` is
  still one key, so nothing that compiled before stops compiling.

  **Each store does what its protocol actually offers**: etcd reads a
  named list as one transaction of range reads and a prefix as one range
  read, both at a single revision; Consul reads a prefix with one
  `?recurse` request and a named list one key at a time, because its KV
  API has no batch read of a caller-chosen set — so a Consul list is *not*
  atomic, and says so; Redis reads a list with one `MGET` and a prefix
  with `SCAN` then `MGET`, never `KEYS`, which blocks a production server
  for the length of its key space; Firestore reads a list with one
  `:batchGet` and restores the reply to call order, because reply order
  must not decide precedence; Vault and NATS read a list one key at a time
  and have no prefix form at all, stated in the `Keys` type rather than at
  run time; S3 lists a prefix with a paginated `ListObjectsV2` whose page
  size *is* the budget check. Git reads a whole directory out of one
  commit's tree.

  **One cost, published rather than discovered.** Provenance becomes
  store-grained: a merged document is one layer, so `source_of` names the
  store and the set it read rather than which key supplied a value.

  **Watching a multi-key source is decided per store, and three network
  stores can now do it.** A watch on a set has to say the set changed *and*
  re-read it as of one instant; waking on `myapp/db` and re-reading key by
  key collects the new `db` beside a half-written `server`, a document that
  never existed. Three protocols answer both questions and now carry the
  loop: **Consul's prefix watch** re-reads nothing at all, because a
  recursive blocking query's answer *is* the subtree at one index;
  **etcd's prefix watch** re-reads one range at the revision the event
  carries; and **Redis' named-list watch** re-reads with the one `MGET` the
  fetch already was, which is a single command and so a single operation on
  the server. Each has a test that stamps one generation into every key,
  changes them all repeatedly, and asserts every delivery agrees with
  itself. The remaining shapes still refuse at `watch()` and point at
  polling, each naming its own reason in the error: **Redis' prefix**
  because re-finding the keys is a `SCAN` and a cursor is many commands,
  **etcd's and Consul's named lists** because neither protocol has one
  request that covers a caller-chosen set, and NATS, Vault, S3 and
  Firestore because they have no atomic re-read to offer at all.

  None of them promises one delivery per write: except for Consul the read
  follows the event rather than being simultaneous with it, so a delivery
  may carry a state newer than the write that woke it and two rapid writes
  may coalesce into one. **Spurious, never torn** — and never older than
  the delivery before it.

  **Untrusted answers are bounded.** A prefix matching more than 512 keys
  is refused rather than pulled into memory; a key the server answers with
  that is not under the prefix asked for is refused rather than merged;
  Redis' `SCAN MATCH` pattern escapes the prefix's glob metacharacters so
  a tenant id with a bracket in it selects itself and nothing else; and
  one unreadable key out of five fails the whole fetch, because a
  configuration quietly missing a section is worse than a refresh that
  failed and left the last one serving.

- **`dynamic-config-git`: a git repository as a store.** GitHub, GitLab,
  Azure DevOps, Gitea, Bitbucket and a bare `git@host:repo.git` all speak
  git, and their file APIs do not speak anything in common — so one client
  on the git protocol covers every host where five REST clients would have
  covered five. Built on `gix`: pure Rust, so the workspace still has no C
  dependency and no OpenSSL question. A fetch is shallow, single-ref and
  never a checkout; an unchanged ref reads the ref advertisement and
  transfers no objects at all, which is what makes polling a git host at
  configuration cadence defensible. A branch is the default ref, because a
  store whose reason to exist is that the configuration changes should not
  default to a ref that cannot; a tag or a full commit id pins it when
  reproducibility matters more than reload. **Every credential can come
  from a callable** — an installation token lives an hour and a watcher
  lives for the life of the process. Objects live in a `0700` directory,
  temporary by default; nothing from the repository is ever written to the
  filesystem, so a tree carrying a symlink to `/etc/shadow` is an error
  rather than a read.

  It is also the store whose **multi-file source can be watched whatever
  shape it takes** — one path, a list or a whole directory. A watch on a set
  needs the store to say the set changed and the set to be re-readable as of
  one instant; a git fetch resolves one commit and reads every path out of
  that one tree, so the second half is free rather than arranged for. Three
  network stores manage it in a narrower shape (see below); four cannot at
  all. And the working directory stops growing: `compact_after` empties the
  object database every thirty-second transfer, deleting only what this crate
  wrote, only in a directory it created, and only on a trigger the caller can
  turn off.

  `with_timeout` reaches further than a git client's usually does. `gix`
  bounds a fetch with an interrupt flag it checks between packets, which is
  a real deadline for the part that transfers data and none at all for a
  host that accepts the connection and then sends nothing — so on the
  transport `tls` installs, the caller's number is now the connect deadline
  and the stall deadline on every read as well. On `gix`'s own transport it
  cannot be: that backend hardcodes twenty seconds for connecting and reads
  none of the timeouts its options type carries, and routing every
  `https://` source through this crate's client to fix it would have cost
  redirect following for callers who never asked for either. Which
  transport bounds which phase is a table in the book instead.

- **`dynamic-config-server`: configuration over HTTP, in the spirit of
  Spring Cloud Config Server.** One resolved document per application and
  profile, served to a caller whose credential is scoped to that
  application. Eight endpoints: the document (the one that returns
  values), `paths`, a redacted `explain`, `check`, `status`, `/metrics`, a
  change stream, and unauthenticated `/healthz` and `/readyz` that say
  nothing else. It **refuses to start** rather than start permissively —
  no clients, a token under 32 characters, an anonymous client without an
  explicit opt-in, two clients sharing a token, or a non-loopback bind
  with neither TLS nor an acknowledgement are each a refusal naming the
  key that fixes it. It **refuses to be an oracle**: a section the caller
  may not read and one that does not exist are the same 404, the same
  body, and the same work. It is a *user* of the library rather than a
  second implementation of it — each served section is a `Dynamic`, with
  the same loader, the same watcher and the same keep-serving-the-last-
  good-document behaviour.

  **TLS termination and mutual TLS**, behind a `tls` feature that is off
  by default, so a deployment with a terminator in front installs a binary
  carrying no TLS code. A client certificate is an *additional*
  requirement rather than an identity: the bearer token still produces the
  principal and still carries the grants, because handing authorisation to
  whoever holds the CA key would create a second roster whose disagreement
  with the first is silent.

  **A change stream** (`GET /{application}/{profile}/stream`) that carries
  a *generation* rather than a document. That is what makes the three
  hard parts disappear: `Last-Event-ID` becomes a comparison rather than a
  replay buffer, memory per connection is flat and independent of document
  size, and backpressure needs no policy because the stream carries a
  level rather than a log.

  **And the client half**, behind a `client` feature:
  `client::ConfigServer` is a `RemoteSource` reading the document endpoint,
  with a bearer token and the same `TlsConfig` the store crates take. Both
  halves in one crate means they are tested against each other on a real
  socket — including the property the client exists for, that killing the
  server mid-run does not take its clients with it, because the
  last-known-good cache that keeps them serving is the *client's*.

- **The Rust remote stores, as an opt-in second wheel.** `pip install
  dynamic-config-py[remote]` buys all eight compiled clients, imported as
  `dynamic_config.remote`. A pip extra installs distributions and cannot
  turn on a Cargo feature in a binary compiled weeks ago, so it resolves
  to a distribution of its own; the base install is unchanged, and
  importing `dynamic_config.remote` without it raises an `ImportError`
  naming the extra. **Every credential argument accepts a callable**,
  resolved on every fetch, because a watcher outlives its credentials.

  The import-name question was settled by measurement: a namespace-package
  overlay builds, but an *editable* base install — `maturin develop`,
  which is how this package is developed — makes it invisible, so the
  remote wheel ships `dynamic_config_remote` and the base wheel carries a
  `dynamic_config/remote.py` that re-exports it.

- **git reaches the Python wheel: `Git`, `GitAuth` and `GitKeys`.** The
  eighth store, and the only one there whose *set* of files is a set: one
  fetch resolves one commit and a commit has one tree, so a list of paths
  (merged in call order, later wins) or a whole directory (disjoint
  sections, an overlap refused) is read as of one instant. The other seven
  still read a single key, because for them a set would be one request per
  key and a document that never existed. The ref is `branch=`, `tag=` or
  `commit=`; naming two is refused, because three keyword arguments have
  no order where Rust's three builder calls do.

  **A rotated token rebuilds nothing.** The credential is a slot the fetch
  path writes and the source reads — S3's shape, for a different reason: a
  `GitSource` owns an object database, and rebuilding one would re-transfer
  the repository's whole tree for a store whose headline property is that
  an unchanged ref transfers nothing. An installation token that lives an
  hour is therefore an ordinary callable.

  Three more refusals, each naming the call and the way out and none of
  them quoting the url — a git remote url routinely carries a token and the
  redaction lives in Rust: an https credential on an ssh url, an ssh
  credential on an https url (silently anonymous otherwise), and `tls` on a
  url with no TLS in it. `watch()` stays unexposed for every store, git
  included, and the book says why it costs more there than anywhere else.
  The wheel went from 8.69 MB to 11.68 MB, `gix` and the three engine
  format features the multi-file fold needs.

  **One example per store**, replacing the three that grouped them:
  `01_etcd` through `08_git`, with
  `09_private_ca_and_client_certificate` staying cross-cutting because one
  TLS vocabulary across eight stores is the claim it demonstrates. Every
  one runs to completion with nothing listening, and each was checked
  against a real server — which is how two errors in the TLS example's
  setup instructions were found: `openssl` does not create `/tmp/tls`, and
  it writes a private key `0600` and owned by you, which the Vault image's
  unprivileged user cannot then read.

- **`RemoteSource` written in Python.** A store with no Rust client — a
  company's own service, a file a sidecar writes — is a class with
  `fetch()` and `describe()`, in the base wheel, needing nothing extra.
  The GIL concern that made this wait was measured and did not hold: an
  I/O-bound `fetch()` releases the GIL itself, so other threads run at
  68–102% of their free-running rate. A `fetch()` may read the
  configuration it is fetching for; only a nested `refresh_remote()` is
  refused, by name.

- **Free-threaded CPython.** The module declares `Py_mod_gil =
  Py_MOD_GIL_NOT_USED` and the suite runs on a real 3.14 free-threading
  build, ten times over for the threading and shutdown suites. The audit
  behind it is a book page of its own: no `static` items in the binding,
  no `unsendable` `#[pyclass]`, and two predictions that measured false.
  Wheels are `cp314t` manylinux `x86_64`/`aarch64`; `cp313t` does not
  exist, because PyO3 0.29 dropped it when CPython promoted
  free-threading to supported in 3.14.

- **Configuration operations: `SnapshotMeta`, `ReloadReason`,
  `ReloadEvent<T>` and `status() -> ConfigStatus`.** *Which generation is
  live, how long ago it landed, why it reloaded, and is it healthy* had no
  answer in Rust. Five reasons, each produced by exactly one path and
  tested there — `Initial`, `FileChanged(path)`, `RemoteChanged`,
  `Manual`, `Recovered` — reach a hook through `on_reload_with(|event|
  ..)`. `on_reload` is untouched: same signature, same contract, still
  silent for the first install. None of it is on the read path: `current()`
  is still one atomic load and does not consult any of it.

- **A `telemetry` feature, and what a remote fetch reports about itself.**
  `telemetry::Exposition` renders `ConfigStatus` and the new
  `RemoteStatus` as Prometheus text, with **no dependency at all** — an
  exposition format is a wire encoding, not a crate, and a library that
  pinned a metrics ecosystem would pick a fight with every application
  that chose a different one. Under `tracing`, an install is a
  `dynamic_config.reload` span and a fetch is a `dynamic_config.fetch`
  span. The metric names are API and the label sets are bounded: six
  families per configuration and six per remote source, none of them
  labelled by anything the crate read — a store's only name for itself is
  its address, and an address routinely embeds a password.

- **A watch loop's failed attempts reach `RemoteStatus`:
  `reporting_to(sink)` on `dynamic-config-etcd` and `dynamic-config-nats`.**
  `RemoteSink::apply` records a delivery, so a *working* watch kept the
  status current — but a loop whose stream broke, whose watch was cancelled
  or whose credential was refused delivered nothing and reported nothing, so
  `dynamic_config_remote_up` described the last *delivery* rather than the
  last *attempt* and a store that stopped answering an hour ago looked
  healthy until something called `refresh_remote`. One builder option per
  store rather than a second `watch` method, over `RemoteSink::failed` and
  the `Attempts` newtype: the watch signatures already differ across the
  family, and a sink is `Copy` and captured at wiring time anyway. A failure
  moves the streak and the last failure and nothing else, so the staleness
  clock keeps ageing while `remote_up` goes to zero — the pair an alert
  wants — and reporting is infallible, because a loop must never have to
  handle a failure to report a failure. **etcd's replaced auth token is
  deliberately not a failure**: the store answered, the resumed stream lost
  no event, and reporting it would hold `remote_up` at zero on a healthy
  cluster until the next change; a re-authentication that *fails* reports.
  `fetch` is untouched everywhere — a fetch already records itself through
  `Remote::refresh`.

- **The same `reporting_to(sink)` on the three HTTP stores:
  `dynamic-config-consul`, `dynamic-config-vault` and
  `dynamic-config-firestore`.** Every failure site in those three loops now
  reports: Consul's blocking query that errored, its watched key that holds no
  value and its subtree that cannot be folded into a document; Vault's
  metadata check, its version that moved beside a secret that will not be
  read, and the v1-mount refusal on its way out; Firestore's failed poll and
  its missing-`updateTime` refusal, likewise on the way out — because a watch
  that has ended is a configuration that has stopped updating for good, which
  is the last thing that should read as a healthy store. **A poll is where
  this matters most**: Vault reads a version counter and Firestore compares an
  `updateTime`, so a secret nobody rewrote and a Vault that sealed itself
  yesterday deliver exactly the same nothing. A source built without the
  option records nowhere and pays for no branch, and only an `ErrorKind` and a
  key path ever reach a `RemoteStatus` — never a store's address.

- **The same `reporting_to(sink)` on `dynamic-config-redis` and
  `dynamic-config-s3`.** Redis reports **both** of its watch failures, and the
  streak is what tells them apart: a re-read that came back with nothing — one
  `MGET`, so a member of the set that went missing or a credential the server
  has started refusing — is transient, and one delivery clears it; a dead
  subscription ends the watch on a thread whose result is usually dropped,
  which is exactly how configuration silently stops updating. S3 reports both
  halves of its poll: the `HEAD` that did not answer, and the `GET` that did
  not answer after the ETag moved — the second being the easier to miss, since
  surviving a failed read is what makes a poll loop robust and also what makes
  it silent. **Refusals at the door are deliberately not reported** in either
  crate — a prefix that cannot be watched, a key naming no format, keyspace
  notifications switched off — because `watch()` returns those to the caller
  standing there, before there is a loop to be silent in, and they are
  deployment mistakes rather than a store that stopped answering.

- **`Builder::secrets_dir(path)`: a directory of single-value files.**
  Docker and Kubernetes mount secrets as a directory — one file per key,
  the filename is the key — and this crate had no way to read one. The
  layer sits above the files and the remote store and below `.env` and the
  environment: a mounted secret is a fact about *this* deployment.
  Provenance is per file, values arrive as strings always, and symlinks
  are followed but not descended into, which is what makes a real
  Kubernetes mount work.

- **Key aliases across sections.** `alias("db::timeout", "timeout")` moves
  a path that changed *section*, not just name. The destination declares
  it, because a source-declared alias is only in effect if that type's
  registration ran first — and in this exact migration the field has just
  been deleted from the source type. It fills a gap rather than
  overriding, like every other alias, and the old key stops counting as
  unknown in its own section without becoming a supported spelling.

- **`Builder::init_and_current()`** and its async twin: the install and
  the snapshot it installed, in one call, so a reload landing immediately
  afterwards cannot change what the caller got.

- **A public parse-and-merge seam**: `Value::parse(text, format)`,
  `merge`, `overlapping_paths` and `render`, with no figment type in any
  signature. It is what lets a store crate fetch N keys, merge them
  later-wins and hand the loader one document without reimplementing
  parsing or growing three format dependencies.

- **`tests/shuttle.rs` and `fuzz/`.** Shuttle covers what loom cannot:
  `arc-swap`, process-wide statics, and three-thread schedules — 200 000
  schedules in the gate from a fixed seed, 8 000 000 in the soak.
  Coverage-guided fuzzing covers the unit parsers, `Value` path lookup,
  the redaction rule, the `.env` parser and the section mapper; the
  section target found a real path-traversal bug within a minute of its
  first run.

- **`dynamic-config-embedded`: `ConfigCell::waiter_evictions()`.** Past
  the waiter budget the failure was measured to be a *livelock*, not
  wake-churn — nine tasks on eight slots never reach idle. Raising the
  default only relocates the cliff, and an intrusive node costs more RAM
  per waiter than a slot does, before the `unsafe`. So the budget reports
  when it is set wrong instead.

- **TLS as data, in all eight store crates.** A custom certificate
  authority and a client certificate (mTLS) — each as a file path or as PEM
  bytes — through one type, `dynamic_config_store_core::tls::TlsConfig`,
  re-exported by `dynamic-config-{etcd,consul,vault,nats,redis,s3,firestore}`
  and by `dynamic-config-git`, which speaks it over `https://` only: an
  `ssh://` remote's trust is `known_hosts` and its identity is a key rather
  than a certificate.
  No client type appears in any signature, which is the point twice over: an
  enterprise behind a private CA can now reach the four stores that had no
  door at all, and the surface is data, so the
  [remote wheel](https://dynamic-config-rs.github.io/python/remote-stores.html) finally has something to
  bind to.

  `Vault`, `Consul` and `Firestore` take it as `.with_tls(..)`; `Etcd`,
  `Nats`, `Redis` and `S3` take it as a constructor argument, because their
  clients want TLS at connect time. Two stores cannot express all of it and
  **refuse the part they cannot**, naming the call and what to use instead:
  NATS has no byte-taking door (`async-nats` opens the files itself) and S3
  has no client-certificate slot (the AWS SDK's TLS context is a trust store
  and nothing else). A silently ignored `ca_certificate` is a program that
  believes it is pinned and is not.

  The per-client escape hatch — `with_options`, `with_agent`, `from_client`,
  `with_config` — is untouched and still the answer for anything this has no
  spelling for; where both doors reach the same slot the interaction is
  defined rather than guessed. **There is deliberately no way to turn
  verification off**, and the argument for that is in the book: it could not
  be uniform, it answers nothing trusting the authority does not, and every
  client underneath still has its own switch under its own frightening name.

  The private key never reaches a log, an error or a `Debug`: renderings are
  shape-only, and no PEM parse error is wrapped, because `rustls-pki-types`
  prints the line it choked on. No new crate enters any build; `rcgen` and
  `rustls` are dev-only, for the self-signed authority the tests generate.
  Two runnable examples, `vault_private_ca` and `etcd_client_certificate`.

- **TLS reaches the Python wheels.** `dynamic_config.remote.TlsConfig` is
  the store crates' `TlsConfig`, method for method, and every one of the
  seven Python stores takes it as `tls=`. This is what the data-only design
  above was for, and it is now demonstrated rather than asserted: a surface
  made of paths and PEM bytes has a Python spelling, and one made of
  `tonic` and `ureq` types does not. The two partial refusals cross intact —
  `Nats` takes certificate paths and not bytes, `S3` takes no client
  certificate — as a `ValueError` at construction naming the call and the
  way out, because a binding that ignored either would be a security bug.
  The wheel enables `dynamic-config-etcd/tls` and `dynamic-config-redis/tls`,
  without which those two would have no TLS constructor to call, and
  `https://dynamic-config-rs.github.io/python/limitations.html` no longer lists TLS as a capability the
  Rust crates have and the wheels do not. Custom proxies and `watch()`
  still are. A runnable example,
  `dynamic-config-python-remote/examples/09_private_ca_and_client_certificate.py`.

- **Instruction counts, under callgrind, as a regression gate.**
  `dynamic-config/benches/instructions.rs` measures five things the crate
  makes a claim about *cost* for, and `.github/workflows/instructions.yml`
  runs them. Wall-clock benches stay ungated because a shared runner's
  variance is larger than the regressions worth catching; instructions are
  the same number on the same binary, so they can gate.

  **The numbers exist now rather than being a design.** On rustc 1.97.1 /
  valgrind 3.24.0 / glibc 2.43 / x86_64: a read is 85 instructions, a
  thousand reads are 75,023 — seventy-five each, with two extra RAM hits
  for the other nine hundred and ninety-nine, which is the no-allocation
  claim as arithmetic. A one-document load is 20,942, `explain` on one key
  is 52,523, and reloading a twenty-key document is 183,791. Four of the
  five reproduce to the instruction across repeated runs and a rebuild;
  only the reload drifts, by under 0.1 %. The limits — 2 % on the read
  path, 10 % on reload, 25 % on `explain` — live in the bench file rather
  than the workflow, because they are a claim about the code, and they are
  wide relative to that measured spread on purpose: the noise that matters
  is the CI image's and it has not been measured yet.

  **The gate is not armed until a baseline is committed**, which has to be
  produced on the CI image — one made on a laptop would fail every run for
  reasons nobody changed. Without one the workflow measures, warns that it
  cannot fail, and uploads the baseline for a maintainer to commit;
  CONTRIBUTING.md has the procedure, and now also how to build valgrind
  into a prefix without root.

- **A document need not be sectioned.** `whole_document()` — on the Rust
  builder, on the Python configuration and its decorator, and as
  `whole_document = true` on a config-server section — reads a file whose
  whole contents are the configuration: `{"host": "0.0.0.0", "port": 8000}`
  with no header above it. The key goes on naming the environment prefix,
  the cache entry and the diagnostics.

  The book gained a [Document Shape] chapter for it and for the three
  questions beside it that nothing answered in one place: a key the file
  has and the type does not name, two files holding half a struct each, and
  a field no source supplies. Every answer has a test and a runnable
  example — `document_shape` in Rust, `19_document_shape.py` in Python.

- **Python: a configuration with no schema class.** `Values` is the
  binding's `Dynamic<Value>` — pass it where a model goes and reads are
  by dotted path. It reports what it cannot know rather than assuming it:
  `Report.unknown_checked` is `False` with no field list, and a redacting
  cache is refused until `secrets=` names the paths. See
  `dynamic-config-python/CHANGELOG.md`.
- **Python: every compiled method documents its parameters.** The
  extension's methods carried no docstrings at all, so `help()` showed a
  signature and nothing else; the decorator's arguments are now listed
  one per row with the fluent call each stands for.

### Changed

- **A new internal crate, `dynamic-config-store-core`, holds what the
  seven store crates had more than one copy of** — the credential cache
  and its refresh margin, the watch callback's panic net, and URL
  redaction. No store's behaviour, public API, error text, retry timing or
  feature flags changed; the existing tests pass unmodified, which is the
  proof. It is published rather than `publish = false` because cargo
  refuses to package a published crate against a path dependency that is
  not on the registry — measured, not assumed. It carries no stable API.

- **`watch::spawn` and `watch::spawn_with` hand their reload closure the
  path that triggered it**: `impl Fn(&Path) -> Result<Option<String>,
  Error>` where it was `impl Fn() -> ..`. That path is the only place the
  changed file is known — the debounce collapses a flurry into one reload
  and used to drop it — and it is what `ReloadReason::FileChanged`
  carries. **Migration:** `|| ..` becomes `|_| ..`. `Builder::watch`, how
  nearly everyone starts a watcher, is unchanged.

- **`ConfigCell::store_with` returns the `Arc<T>` it installed** rather
  than `()`, which is what makes `init_and_current` able to hand back its
  own install. **Migration:** a `store_with(..)` used as the tail
  expression of a function returning `()` needs a `;`.

- **The last-known-good cache's fingerprint is computed from the
  structural value** rather than from a `Debug` rendering, so a cache
  written by one build is not silently rejected by another whose `Debug`
  spacing differs.

- **`explain`'s alias row names the old path**, through a new
  `Contribution::aliased_from`, so a value that arrived through an alias
  says which spelling supplied it and in which section.

- **`::` in an ordinary key path is an error** naming the alias syntax,
  rather than being read as part of a key.

### Fixed

- **`Remote::clear()` did not bump the generation fence**, so a fetch in
  flight when a caller cleared the slot put the document back when it
  landed. Configuration somebody explicitly dropped came back from a
  network round trip that started before they dropped it.

- **`changed_paths` and `Snapshot::diff` compared *rendered* values**, so
  a reload that changed a number's type but not its value — `1u8` to
  `1u64` — reported a change, and a reload that changed a value figment
  renders identically reported none. They compare the structural value
  now.

- **The concurrent-writer contract on `on_reload` is documented** rather
  than living in a comment, and is pinned by a test.

- **The engine**: `clear_remote()` no longer ends a running watch, and a
  fetch that lands after its source was replaced no longer reports on the
  replacement's health. See `dynamic-config/CHANGELOG.md`.
- **The config server**: a response can no longer carry the previous
  document under the new generation; a stream resumed from a previous
  process is no longer left silent; SIGTERM is handled; shutdown is
  bounded; a TLS connection has a deadline after the handshake; a section
  no route could reach is refused at startup; and on the client side a
  password in the URL reaches no diagnostic, while the fetch deadline now
  covers the response body. See `dynamic-config-server/CHANGELOG.md`.
- **The git store**: a credential in the URL no longer reaches `Builder`'s
  `Debug`, and one working directory can no longer be claimed twice under
  two spellings. See `dynamic-config-git/CHANGELOG.md`.

[Document Shape]: https://dynamic-config-rs.github.io/document-shape.html

### Security

- **A profile variant could leave the directory of the file it varied.**
  `profile_variant` reasoned about the whole path rather than the file
  name, and stripping `.age` off a name like `..age` leaves a trailing `.`
  component whose *file name*, to `Path`, is the directory above it. So
  `/etc/my.app/..age` with `APP_PROFILE=production` resolved to
  `/etc/my.production.app.age` — one level up, in a directory the caller
  never named, which is exactly the traversal the profile rule exists to
  prevent. Directories with a dot in the name (`/etc/my.app`,
  `/srv/conf.d`) are ordinary, so nothing unusual on the caller's part is
  needed to reach it. The naming now happens on the file name alone and is
  rejoined to the original path once, so the sibling rule is structural
  rather than something each branch has to remember. Found by the new
  `sections` fuzz target.

- **Five more roads a configured value could reach a diagnostic on**, all
  found by looking rather than reported. `toml` renders a syntax error by
  quoting the failing line, and an unterminated string is how a pasted
  password becomes a syntax error — so `load()` printed the password.
  `Snapshot::get` and `Snapshot::extract` deserialize a value already in
  hand and never went through the loader's translation, so figment's
  ``invalid type: found string "hunter2", expected u16`` reached the
  caller verbatim. The unit parser quoted the text it could not parse.
  `write.rs` printed a non-table section with `{:?}` on the path about to
  write it to disk. And a NATS URL's credentials reached every error and
  every `Debug`. `loader::origin::translate` is now the one place a
  backend error is stripped, and `tests/security.rs` grew to 27 cases.

- **A private key never reaches a diagnostic**, anywhere in the new TLS
  surface. Every type that can hold one prints shape only, no PEM parse
  error is wrapped — the one field such an error has is the input it choked
  on, and in a key file that input is key material — and the config
  server refuses at startup to read a private key file that anything but
  its owner can read, naming the fix and the Kubernetes case that produces
  it (a secret volume mounts `0644` by default).

- **There is no `skip_verification`**, in any store or in the wheels, and
  the refusal is argued rather than assumed: a self-signed development
  server and an enterprise CA are both *one more trusted certificate*,
  which `with_ca_certificate_file` expresses while leaving the server
  authenticated. For the git store the argument is sharper still — a fetch
  presents its credential before it has received anything, so an
  unverified connection hands the token to whoever terminates it. Each
  client keeps its own switch under its own frightening name, reachable
  through the escape hatch where a reviewer sees it.

- **The config server refuses a `[server.tls] crl` rather than shipping a
  revocation check that does not check.** It carries no CRL, and the key
  exists only so an operator who reaches for revocation is told that,
  instead of serde answering `unknown field`, which reads as a misspelling.
  Measured against rustls rather than read off its documentation, and kept
  executable in
  `dynamic-config-server/tests/tls.rs::the_measurement_behind_refusing_revocation_still_holds`:
  by default a CRL whose `nextUpdate` passed in 2020 verifies a handshake
  today with no error and no log line, so the twenty obvious lines produce a
  server that reports it checks revocation and stops doing so, invisibly,
  the moment the file stops being refreshed — and that version *tests
  green*, because revoking a certificate and asserting the handshake fails
  passes against a six-year-old list. The one switch that refuses a stale
  list refuses every clean client with it, making a CA's publishing cadence
  a liveness dependency of every service's configuration fetch. Re-reading
  the file on the section watcher does not rescue it: a watcher fires on a
  write, and what has to be noticed is the *absence* of one. Since a
  certificate there is a gate and not an identity — a stolen one buys a TCP
  connection and a 401 — a CRL would revoke the credential that does not
  authorise while the one that does is a line in a file the operator already
  holds. Issue short-lived certificates and revoke the token.

## [0.5.0] — 2026-08-12

### Added

- **`dynamic-config-py` — the Python bindings.** A PyO3 extension pairing
  this engine with Pydantic: Rust owns sources, layering, watching,
  recovery and provenance; Pydantic owns the schema; Python reads a
  cached model for the price of an attribute lookup. Validation runs once
  per successful resolve, a reload Pydantic rejects keeps the previous
  model serving, and the secret list is derived from the model's own
  `SecretStr` fields. Ships to PyPI as `dynamic-config-py`, on a version
  of its own — the wheel embeds the engine rather than depending on a
  published version of it, so it does not move every time the crates do
  — and the import is `dynamic_config`. See [its chapter](book/src/python.md). Reading is an
  attribute lookup — 28 ns against a module global's 20 — because the
  model is published into the Python object as it installs;
  `changed_paths` gives the audit half of a reload from Python too.
  Every blocking call has an async twin (`init_async`, `load_async`,
  `reload_async`, `changed_async`) and an executor knob to choose which
  pool pays for it, thirteen runnable examples ship with the package, and the chapter has its own sections for
  [async](https://dynamic-config-rs.github.io/python/async.html),
  [data types](https://dynamic-config-rs.github.io/python/types.html),
  [web frameworks](https://dynamic-config-rs.github.io/python/frameworks.html) and
  [limitations](https://dynamic-config-rs.github.io/python/limitations.html).
- `Builder::validate` accepts closures, not just function pointers: a
  validator that needs *context* — a policy object, a schema, another
  runtime's validator — could not be written as a `fn`, and that is the
  shape a language binding needs. A plain `fn` still coerces, so every
  existing call site is unchanged.
- `touches_secret(path, secrets)`, the one rule every redaction door
  asks: a path that *is*, sits *under*, or *contains* a secret is
  redacted.

### Fixed

- **Nested secrets were redacted nowhere.** A secret named by a dotted
  path — `credentials.password`, which is what a nested model produces —
  was missed by both redaction doors: `explain` matched only the head of
  the path, and the redacted last-known-good cache dropped only
  top-level keys. Both now understand dotted paths, so a secret inside a
  nested table stays out of the cache file and reads `***` in an
  explanation, whether the path asked about is the secret, something
  under it, or the table containing it.

## [0.4.0] — 2026-08-12

### Added

- **The figment review's fixes landed.** Top-level tables named `global`
  or `default` are ordinary sections now: sections ride on a *namespaced*
  profile, so figment's reserved-profile inheritance — which silently let
  a `global` table override every section's own values, invisible to
  `check` and `source_of` — has nothing to grab. And environment
  provenance names the exact variable (`APP_DB_POOL__MAX_SIZE`), derived
  from prefix, path and the nesting separator, instead of `APP_DB_*` —
  in errors, `source_of`, snapshot provenance, `check` and `explain`
  alike.
- **`cache_encrypted(path, encryptor)` — the last-known-good cache,
  encrypted at rest.** Full fidelity with nothing readable on disk:
  written through the caller's `Encryptor`, recovered through the
  installed `Decryptor` — the same door `encrypted_file` reads through.
  Behind the `decrypt` feature.
- **`dynamic-config-cli` is on crates.io** (`cargo install
  dynamic-config-cli`), with `completions` and `man` subcommands — and
  `explain` now redacts by default; `--show-values` opts in (the CLI's own
  changelog carries the details).
- **`Dynamic<T>` — the instance engine.** One configuration per *value*
  rather than per type: its own snapshot, hooks, watcher and cache, with
  the same builder carrying the sources. `current()` answers `Option`
  instead of panicking, two instances of one type watch side by side, and
  the type-level surface is untouched. The watcher registry now keys on
  `WatchKey` (`Type` or `Instance`) to make that possible.
- **`Value` and `Snapshot::to_value`.** The resolved tree as owned data —
  seven shapes, no loader types in the signature, built by walking the
  resolved tree rather than a JSON round trip — for boundaries that need
  configuration as values: exporters, language bindings.

### Breaking

- `watch::spawn` / `watch::spawn_with` take a `WatchKey` where they took a
  `TypeId` (`WatchKey::Type(id)` is the old behaviour), and the async and
  grouped-commit builder surfaces (`load_async`, `init_async`, `prepare`)
  require `T: Sync` — the builder can now carry a shared cell, and moving
  it to a worker moves the cell with it.

## [0.3.0] — 2026-08-11

### Breaking

- The remote push path is fenced like the fetch path always was:
  `apply_remote(document)` is replaced by `remote_sink()` — taken **once,
  at wiring** — whose `apply` refuses to deliver for a source that has
  since been replaced. A stale watch loop's push now bounces (and, since a
  callback error ends a store watch, the stale loop winds itself down)
  instead of quietly overwriting the store that followed it.

### Added

- The concurrency claims are model-checked: under `--cfg loom` the library
  swaps its sync primitives for loom's (`src/sync.rs`), and `just loom`
  runs the remote fence — fetch and push — and the async wake protocol
  through every interleaving, on the real code. The check-register-check
  dance now lives in one place, `Notify::poll_with`, which is what the
  model drives.

### Security

- The dependency lockfile moved to the patched versions behind every open
  Dependabot alert: `quinn-proto` 0.11.15 (GHSA-4w2j-m93h-cj5j,
  GHSA-6xvm-j4wr-6v98), `actix-http` 3.12.1 (GHSA-xhj4-vrgc-hr34),
  `serde_with` 3.21.0 (GHSA-7gcf-g7xr-8hxj) and `aws-sdk-s3` 1.112.0
  (GHSA-g59m-gf8j-gjf5). Library consumers resolve their own trees and
  were never pinned to the vulnerable versions by these crates; the
  lockfile governs this repository's CI and any `--locked` install. The
  standing triage rule is now written into `SECURITY.md`.

## [0.2.0] — 2026-08-11

### Breaking

- **The attribute declares; the builder configures.** `#[dynamic_config]`
  takes no arguments any more: every source argument moved to the `Builder`
  — `files` → `.file(..)`, `name`+`paths` → `.discover(..)`, `key` →
  `builder("key")`, `env`/`nest`/`allow_empty_env`/`strict_env` →
  `.env(..)`/`.nest(..)`/`.allow_empty_env()`/`.strict_env()`, `env_files`
  → `.env_file(..)`, `profile_env` → `.profile_env(..)`,
  `cache`/`cache_mode` → `.cache(path, mode)`, `validate` →
  `.validate(f)`, `watch`/`debounce`/`poll` → `.watch(debounce)` /
  `.watch_with(debounce, mode)` on the builder `init()` was called on.
  Generated `load`/`init`/`start_watch`/`save*`/`schema` methods are gone:
  loading goes through the builder, `save`/`save_new`/`save_encrypted` are
  the free functions they always also were, and `schema` is
  `builder.schema()`. A successful `init` remembers its builder, which is
  how `source_of`, `check`, `explain`, `prepare`, `apply_remote` and the
  async loaders on the type keep answering. The attribute error for any
  argument is the migration map. The `diff` argument is gone —
  `changed_paths` in an `on_reload` hook is its replacement.
- `watch::spawn` / `spawn_with` take an owned `watch::Watched` (built with
  `Watched::from_spec`) instead of a `LoadSpec<'static>` — the watch no
  longer requires statics only the attribute can produce, which is what
  frees the builder (or anything else) to start one from runtime data.
- The cache is configured on the builder, and the mode is always spelled
  out: `.cache(path, CacheMode::Redacted)`. Secrets on disk are a decision,
  not a side effect — `Redacted` recovers completely when the secrets
  arrive from somewhere live, and the redaction-dependent modes are refused
  on a bare `Builder::new`, which cannot know which fields are secret.

### Added

- `explain(path)` — every configured layer's answer for one path, not just
  the winner's, rendered as a table; the one diagnostic that shows values —
  through `Display`, deliberately: its `Debug` is value-free — and
  `#[config(secret)]` fields stay `***`. Generated on every config type
  and available as `dynamic_config::explain` without the macro.
- `Snapshot::source_of(path)` — snapshots now carry the provenance of their
  own leaves, captured at resolution time; the free `source_of()` keeps its
  next-load meaning, now documented as such.
- `dynamic-config-cli` (Experimental, in-repo, not yet published): `explain`
  and `diff` from a shell, the load restated as flags.
- `strict_env` (`.strict_env()` on the builder, `with_strict_env` on
  `LoadSpec`): the yes/no/on/off family in an environment value (or a
  `.env` file) becomes an error naming the variable, instead of arriving as
  a string where a boolean was meant — and the refusal holds on the cache
  recovery path too. Loose parsing stays the default.
- `Builder<T>`: runtime-chosen sources with the attribute's semantics —
  `Builder::new("db").file(path).env("APP_").load()`, no macro required. On
  a `#[dynamic_config]` type the generated `builder()` adds `init()`, which
  installs into the same snapshot `current()` reads. The builder reaches
  source-side parity with the attribute: `discover(name, paths)`,
  `cache(path, mode)` with last-known-good recovery (redaction-dependent
  modes are refused unless the generated `builder()` supplies the secret
  fields — and `Fingerprint` never recovers, even from a value-bearing
  file an earlier deployment left at the same path), `watch(debounce)`
  through the same one-watcher-per-type registry, and
  `load_async`/`init_async` under the `async` feature.
- Reload observability: under `tracing`, every watcher reload is a
  `config_reload` span with outcome and duration; the stderr lines carry
  the duration without it. `changed_paths(old, new)` names what moved
  between two configuration values — paths only, never values — for audit
  logging inside `on_reload`.
- The book gains [The Reload Lifecycle](book/src/reload-lifecycle.md):
  where the crate's half of a reload ends, and the surface for yours — and
  [The Builder, Feature by Feature](book/src/builder-tour.md): every
  capability with a minimal example, files to callbacks to hot reload.

### Changed

- `changes()` before `init()` is now contract: the handle has seen nothing,
  so the initial install is its first change — "wake me when configuration
  exists". The behaviour is unchanged; it is now documented and tested.

### Fixed

- `Snapshot`'s `Debug` printed the resolved values — secrets included — and
  so did `Recovery`'s through it. Both now describe keys and shape only,
  with a test that plants a secret and greps the output.
- The `strict_env` refusal does not echo the offending value; it names the
  variable and the ambiguous family. No diagnostic prints a value, without
  exception.

## [0.1.0] — 2026-08-10

A full-workspace hardening release, driven by a three-way external review.
Breaking changes are deliberate and none of them should survive silent
misbehaviour: every one replaces a quiet wrong with a loud right.

### Breaking

- `Recovery::Drift` carries `Option<Vec<String>>`: `Some` is the fingerprint
  (key paths, or one explanatory sentence when only values moved), `None`
  means the comparison itself was impossible.
- `start_watch()` on a type that is already being watched returns
  `Err(AlreadyExists)` instead of a success handle that owned nothing.
- Watchers are keyed by `TypeId`: generic configurations get one watcher per
  instantiation — `Db<Postgres>` and `Db<Mysql>` no longer silently share
  (and lose) one. `watch::spawn`/`spawn_with` take the `TypeId`.
- Async waiters (`changes()`) are woken *before* reload hooks run, so a slow
  hook no longer delays every async reader.
- The empty-environment rule is unified: whitespace-only counts as empty,
  and `allow_empty_env` is honoured by env-var bindings too.
- Last-known-good cache files now carry a `__dynamic_config_cache` marker;
  files written by 0.0.x are still read via a fallback that will be removed
  in the next minor.
- A remote fetch whose source was replaced mid-flight is discarded instead
  of pairing the new source with the old store's document.
- Embedded: `ConfigCell` gains a const-generic waiter budget
  (`ConfigCell<T, 8>`); `WAITERS` is now `DEFAULT_WAITERS`.
- Store watch callbacks that panic end the watch with an error instead of
  killing the calling thread.

### Added

- `on_reload_scoped` → `HookGuard` (drop to unsubscribe); hooks are now
  panic-isolated.
- `set_defaults(&struct)` seeds the defaults layer from a whole struct.
- Alias chains resolve deterministically; cycles are refused at `add`;
  a runtime default no longer defeats an alias.
- Atomic writes fsync before rename (and the parent directory on Unix).
- mdBook documentation, published to GitHub Pages.

### Fixed

- `{:?}` on Vault/Consul/Firestore sources — and on `Fetched` — no longer
  prints credentials or document contents.
- A non-UTF-8 variable anywhere in the environment no longer panics
  `load()`.
- Recovery honours `validate`, seeds the diff baseline, and keeps the real
  environment above `.env` files — all exactly as a normal load does.
- `#[serde(rename)]`d (and `rename_all`'d) secrets are redacted in the
  redacted cache under the names the files actually use.
- A profile from the environment must be a plain word — `APP_ENV=../x`
  is refused, not interpolated into a file path.
- The file watcher filters events before debouncing and bounds the wait, so
  a chatty neighbour cannot starve reloads.
- Redis: a dead subscription ends the watch with an error instead of a
  silent busy-loop; URL redaction survives passwords containing `@`.
- etcd: an expired auth token mid-stream re-logs-in and re-establishes the
  watch instead of failing terminally.
- Embedded: trailing bytes after a JSON document are rejected; the evicted
  waiter really is woken.

## [0.0.1] — 2026-08-10

The first release: ten crates, versioned together.

### Added

- **`dynamic-config`** — hot-reloadable configuration behind one attribute:
  layered loading (defaults < files < remote < `.env` < environment <
  bindings < flags < overrides), lock-free reads, file watching with
  debounce, an async surface that names no runtime, remote stores, a
  last-known-good cache, transparent `age` decryption, JSON Schema export,
  provenance (`source_of`), and diagnostics that never contain values.
- **`dynamic-config-macros`** — the `#[dynamic_config]` attribute with twenty
  arguments, every misuse a compile error pointing at the offending token.
- **Seven store crates** — etcd, Consul, NATS, Redis, Vault, S3 and
  Firestore, each a separate crate so one store's dependency tree never
  reaches a build that did not ask for it. All seven watch, all seven are
  tested against real servers in containers.
- **`dynamic-config-embedded`** — the same shape for `no_std` targets: a
  snapshot in a `static`, validation before installation, `changes()` as a
  plain `Future`. No allocator, no runtime, no code shared with the rest —
  deliberately.

[Unreleased]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.1...HEAD
[0.6.1]: https://github.com/ctolon/dynamic-config/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/ctolon/dynamic-config/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

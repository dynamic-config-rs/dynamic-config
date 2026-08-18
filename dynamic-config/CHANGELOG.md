# Changelog

All notable changes to `dynamic-config` are documented here. The format follows
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

## [0.7.0] — 2026-08-18

## [0.6.3] — 2026-08-17

## [0.6.2] — 2026-08-16

## [0.6.1] — 2026-08-14

### Changed

- **The cache refusal spells the second way out in Python too.** It already
  translated `.secrets([..])` into `DynamicConfig(..., secrets=[..])` and then
  offered `CacheMode::Full` — a name Python does not have. It now names
  `cache(path, "full")` beside it, so both ways out of the refusal are
  readable by whoever hit it.

## [0.6.0] — 2026-08-13

### Added

- **`RemoteSink::failed(&error)`: the door a watch loop reports a failure
  through.** `apply` records a delivery, so a working watch keeps
  `RemoteStatus` current — but a loop whose stream broke or whose credential
  was refused delivers nothing and said nothing, which left
  `dynamic_config_remote_up` reporting the last *delivery* rather than the
  last *attempt*: a store that stopped answering an hour ago looked healthy
  until something called `refresh`.

  It moves the failure streak and the last failure, and deliberately nothing
  else — `fetches`, `last_fetch` and `last_fetch_duration` are left alone, so
  the staleness clock keeps running while `up` goes to zero, which is the
  pair an alert wants. Fenced on the sink's generation exactly as `apply` is,
  so a loop winding down after its source was replaced cannot charge its
  failures to the replacement, and silent rather than fallible: a loop must
  never have to handle a failure to report a failure.

- **What a fetch from a remote store reports about itself.** `Remote` now
  records a `RemoteStatus` — fetches, when the last document arrived, how
  long the last *pull* took, the last failure's `ErrorKind`, and the
  failures since one succeeded — and `reachable()` answers `Some(true)`,
  `Some(false)` or **`None` before the store has been asked anything**,
  because a source that has been installed and never fetched is not down.
  It is the fetch half of the picture `ConfigStatus` starts, in the same
  vocabulary rather than a second one: the same `FailureStatus`, the same
  zero-is-healthy counter, and the same `Exposition`, through `add_remote`
  and `add_remote_with`. The distinction is the point — *did the store
  answer* here, *did the document install* there — and a fetch returning
  an unchanged document is the case neither surface could report before.
  Six more families: `dynamic_config_remote_up`, `_fetches_total`,
  `_last_fetch_seconds`, `_last_fetch_duration_seconds`,
  `_consecutive_failures` and `_last_failure_info`. **These names are
  API.** With `tracing`, a pull is a `dynamic_config.fetch` span *around*
  the round trip with an event inside carrying `outcome` and, on a
  failure, `error.kind`.

  **No label and no field names the store.** The only string a source has
  for itself is its URL, and a store URL routinely embeds
  `user:password@host` — so a series is named by the caller, with the same
  label its `ConfigStatus` carries, and the two halves join in a query.
  The bound is stated: for `C` configurations and `R` remote sources,
  `6 × C + 6 × R ≤ 12 × C` series per scrape and `34 × C` over a process's
  life. A document that arrives by *push* counts as a fetch and reports no
  duration, because the store crate that made the round trip is the one
  that timed it. Nothing is on the read path, and `RemoteSink::status()`
  is the door a `#[dynamic_config]` type reaches all of it through.

- **A configuration with no struct behind it, reading by path.** `Value`
  now implements `Deserialize`, and `DeserializeOwned` is the only bound
  the engine puts on a configuration type — so `Dynamic<Value>`,
  `Builder::values(key)` and `load::<Value>(..)` are a schemaless
  configuration with everything else intact: the same layers, profiles,
  discovery, `secrets_dir`, watcher, last-known-good cache, reload hooks,
  `source_of` and `explain`. Nothing in the engine ever knew what `T`
  was. (Remote stores and the runtime layers are the one exception, and
  not a schemaless one: they live in a `#[dynamic_config]` type's
  statics, so no bare builder reaches them whatever `T` is.) Reading is
  `Value::get(path) -> Option<&Value>` plus `as_str` /
  `as_i64` / `as_u64` / `as_integer` / `as_float` / `as_bool` /
  `as_array` / `as_table`, with `get_as::<T>(path)` for a serde type the
  accessors cannot express and `leaf_paths()` for the keys a program did
  not declare.

  **What a path read costs, since the crate's central claim is that a
  read is an `ArcSwap` load and a field access.** Measured in one run of
  `benches/read_path.rs` on an i7-14700F: a struct field is 19.8 ns, a
  one-segment path 27.2 ns (1.4×), a two-segment path 32.1 ns (1.6×),
  and `get_as::<u16>` 36.7 ns (1.9×). The atomic load is shared and
  unchanged; what a path adds is a `split('.')` and one `BTreeMap`
  lookup per segment, growing with depth rather than with the size of
  the configuration. `benches/alloc_profile.rs` counts **zero
  allocations per 100 000 reads** for the path read and for a typed
  scalar read alike; what allocates is handing back something owned.

  **No Cargo feature and no new dependency.** A `Deserialize` impl behind
  a `#[cfg]` would mean `Dynamic<Value>` compiling in one build of this
  crate and not another, which a dependent library cannot rely on at all.
  `DashMap` was considered and rejected for the same reason the roadmap
  gave: reads are lock-free because the snapshot is immutable and
  swapped whole, so a sharded map would replace a pointer load with a
  shard lock and buy nothing. Per-key writes are a different product.

  **What it does not get, made loud rather than silent.** `Report` gained
  `unknown_checked`, and a report with no field list to compare against
  now renders `unknown keys: not checked (no field list)` instead of an
  empty list that reads as an all-clear. `Builder::secrets(&[..])` is the
  redaction list stated by hand — what `#[config(secret)]` declares for a
  type that has fields to declare it on — and without one, a
  redaction-dependent cache mode is still refused at `init` rather than
  written. `Value` has a shape-only `Debug` and deliberately **no
  `Display`**: a type that rendered itself into `{}` would put a password
  wherever a program formats a value it did not inspect.

- **`Builder::secrets_dir(path)` and `LoadSpec::with_secrets_dir`: a
  directory of single-value files.** Docker and Kubernetes mount secrets
  as a directory — one file per key, the filename is the key, the
  contents are the value — and this crate had no way to read one. One
  directory level; nesting is spelled in the filename with the same
  separator `nest` sets (`db__password` is `db.password`), because that
  is what a mount actually produces and it keeps one setting governing
  this layer and the environment alike. One trailing newline is removed
  and no more, since every tool that writes a secret writes one.

  The layer sits **above the files and the remote store, below `.env`
  and the environment**: a mounted secret is a fact about this
  deployment, so it beats a document a central store hands to every
  deployment alike, and loses to a variable exported for this one run.

  **Provenance is per file.** Each key is its own provider naming its own
  path, so `source_of("password")` answers `/run/secrets/password` and
  `explain` names the file rather than the directory. Redaction is by
  path as everywhere else, so a mounted value under a `#[config(secret)]`
  field reads `***` in an explanation and stays out of a redacted cache.

  A missing directory is skipped, exactly like a missing file — the same
  image has to start in a test that mounts nothing. One that cannot be
  read is an `ErrorKind::Io` error naming the path and never the
  contents; a file whose bytes are not UTF-8 is an error rather than a
  lossy conversion. Symlinks are followed but not descended into, which
  is what makes a real Kubernetes mount work: every key there is a
  symlink into a timestamped directory behind `..data`, and the
  directories themselves are skipped so they cannot contribute the whole
  set a second time.

  Values arrive as **strings**, always. The environment layer parses what
  it reads, which turns an all-digit password into an integer and then
  fails the `String` field it was meant for; a directory of credentials
  is the last place that should happen. The cost is the other direction:
  a numeric field cannot be fed from a mounted file.
- **Snapshot metadata: `SnapshotMeta`, `Dynamic::generation()`,
  `Dynamic::meta()`, and the same two on `ConfigCell`.** *Which generation
  is live, and how long ago did it land* had no answer in Rust — the
  generation counter existed but was `pub(crate)`, and nothing recorded the
  install time at all. `generation()` is the number a reload hook should
  read when it needs a total order; `meta()` adds the monotonic `Instant`
  the snapshot was installed at, so "how stale is this" is a subtraction
  rather than a wall clock that can go backwards. Metadata is deliberately
  **not** on the read path: `current()` is still one atomic load and does
  not consult it, which means the value and its metadata are two loads and
  can be one install apart. It is for operators, not for correctness.
- **A reload knows why it happened: `ReloadReason`, `ReloadEvent<T>` and
  `on_reload_with`.** `on_reload(old, new)` hands a hook two snapshots and
  nothing else, so a file edit, a manual `reload()` and a document a store
  pushed all arrive as the same swap; the reason has to be recorded where
  the reload is *triggered*, and now it is. Five reasons, each produced by
  exactly one path and tested there — `Initial`, `FileChanged(path)` (the
  watcher, naming the file whose event opened the debounce window),
  `RemoteChanged`, `Manual` and `Recovered` — carried to a hook by
  `on_reload_with(|event| ..)` along with both snapshots and the install's
  `SnapshotMeta`. **`on_reload` is untouched**: same signature, same
  contract, same list, still silent for the first install. The event form
  fires for that one too, with `previous: None`, which the pair form's
  signature has nowhere to say. On the generated type, on `Dynamic<T>` and
  on `ConfigCell<T>`, each with a `_scoped` twin; `Builder::reload_with`
  and `ConfigCell::store_with` label an install from the outside. A
  `ReloadEvent`'s `Debug` prints the reason, the generation and *whether*
  there was a previous snapshot, never the snapshots themselves.
- **`status() -> ConfigStatus`.** Which generation is live, when it landed,
  why, and how many reloads have failed since one worked
  (`consecutive_failures`; zero is healthy). A handful of atomic loads and
  no I/O, so an exporter can call it per scrape. It carries key paths,
  counts, timestamps, generations and error kinds and — by construction —
  no configured value: `last_failure` keeps an `ErrorKind` and the key path
  it was reported at, not the message. No `last_success` (an install *is*
  the success, so `loaded_at` is when the last one was) and no source list
  (`check()` answers "what would load" against the sources rather than from
  a cache of them). On the generated type, `Dynamic<T>` and `ConfigCell<T>`;
  `ConfigCell::record_failure` is the door a reload that installed nothing
  reports through.
- `Hash` for `Value`, hashing floats through `to_bits()` — so `-0.0` and
  `0.0` hash differently, which is what a fingerprint of a *document*
  should do.
- **`tests/shuttle.rs`: a second model checker, for what loom cannot
  reach.** `src/sync.rs` grows a third arm — under `--cfg shuttle` the
  library's `Mutex` and atomics are shuttle's, exactly as they are loom's
  under `--cfg loom`. Shuttle's constructors are `const`, so unlike loom's
  arm nothing outside that file changes. Four models, run by `just
  shuttle`: two installs racing into a cold `ConfigCell` (`generation`
  counts both, a reader never sees a value nobody wrote); reload hooks
  registered and dropped while reloads land (no lost `register`, no lost
  `unregister`); two reloaders through one `ReloadGroup` beside a group
  whose member fails (a commit loop is an intact block, and a failed
  prepare commits nothing); and a `static` cell awaited through
  `changes()` (no lost wake-up, and a woken waiter never reads the
  replaced snapshot). The last one is the point: loom drives
  `Notify::poll_with` against a synthetic load closure because the real
  one goes through `arc-swap`, and this composes the real `store` with the
  real `Changes`. Fixed seed by default, so it is a regression test rather
  than a search; `just shuttle-soak` searches, and both print the seed.
- **`fuzz/`: coverage-guided fuzzing for the value and unit surfaces.**
  Three targets — `duration::parse`/`bytes::parse`, `touches_secret`, and
  `Value::get` — the latter two structure-aware, generating path shapes
  and value trees rather than bytes. A separate workspace on nightly, so
  the crate's lockfile and its 1.71 floor are untouched.
- **`Builder::init_and_current()`**, with `init_and_current_async()` behind
  `async` and both on `Dynamic<T>`: `init()`, handing back the `Arc<T>` it
  installed. The two calls always pair at startup, and splitting them means
  naming the type twice. What comes back is *that* install's snapshot — a
  reload landing immediately after moves `current()` and not this — which is
  also why it is not `init()?; current()` under a different name. Off the
  read path entirely: `current()` is the same single atomic load, and the
  install's `Arc` was allocated either way.
- **`HookGuard` is `#[must_use]` as a type.** The attribute was on
  `ConfigCell`'s scoped registrations and the generated ones, but not on
  `Dynamic::on_reload_scoped` / `on_reload_with_scoped`, where
  `instance.on_reload_scoped(|_, _| ..);` registered a hook and dropped the
  guard at the semicolon — a hook that never fires and says nothing. On the
  type it covers every producer, present and future.
- **A public parse-and-merge seam: `Value::parse(text, format)`,
  `Value::merge(other)`, `Value::overlapping_paths(other)` and
  `Value::render(format)`.** `Value` was export-only, so combining documents
  *before* the loader sees them meant taking `serde_json`, `toml` and the
  archived `serde_yaml` as direct dependencies and rewriting parsing this
  crate already compiles. `merge` is later-wins, tables deep, arrays replaced
  whole — the rule the file layers follow. `overlapping_paths` is for the
  caller whose documents are meant to be disjoint: it names the leaves both
  supply, paths only, so the report is safe to print. What a store crate
  reading several keys under a prefix needs to hand `Fetched` one document;
  see the book's *Writing a Store*. No figment type in any of the four
  signatures.
- **`__fuzz`, not public API.** A `#[doc(hidden)]` module reaching the `.env`
  splitter, the section-profile mapper and the profile/filename pair, so this
  repository's fuzz targets can drive them without a temporary file and a
  whole load in between — and without publishing a stability promise for a
  `KEY=value` splitter. Absent from rustdoc and from the book, like
  `__private`.
- **A `telemetry` feature, and a record per reload.** Two halves of one
  item, behind two features, neither of which pulls a crate this workspace
  did not already have. With `tracing`, an install is a
  `dynamic_config.reload` span — `config`, `reason`, `generation`,
  `outcome` — entered around the reload hooks so that whatever a hook logs
  belongs to the reload that ran it, and a reload that installs nothing is
  a `WARN` event carrying the `ErrorKind` and the key path. With
  `telemetry`, `telemetry::Exposition` renders one or more `ConfigStatus`
  values as Prometheus text: `dynamic_config_installs_total`,
  `_last_success_seconds`, `_consecutive_failures`, `_last_failure_seconds`,
  `_last_reload_info` and `_last_failure_info`. **These names are API.**
  `telemetry` has **no dependency at all** — an exposition format is a wire
  encoding, not a crate — so the library still picks no metrics ecosystem:
  an application on `metrics` or OpenTelemetry reads `status()` in its own
  recorder, and the spans bridge through `tracing-opentelemetry`. Nothing
  is emitted on the read path, and with both features off there is no
  module and no code. Neither moves the MSRV. **No label can carry a key
  path, a file name or a value** — one notch tighter than a log line, which
  may name a key — and the series count is bounded: `6 × configurations`
  per scrape, `19 ×` over a process's life.

- **An alias's old path may name the section a key moved out of**:
  `ServerConfig::alias("db::timeout", "timeout")`. **The type that owns the
  key today declares where it used to live**, never the reverse — the
  reverse is a claim on another type's section, resolved only if it was
  registered first, and in this migration the field has just been deleted
  from the type that would make it. `to` may not be qualified, so nothing
  can point at a cross-section alias and it is always the head of a chain:
  one hop, by construction. Precedence, typo detection and provenance are
  the in-section rules unchanged — it fills a gap rather than overriding,
  `db` does not become a legitimate key of `[server]`, and `source_of`
  names the file holding the old spelling. The other section is read from
  **this configuration's own documents**, which every source already parses
  whole: no second file list, no second read, nothing to cache, and no
  watcher blind spot. Its environment, defaults, flags and overrides are
  not consulted — those are built from this load's key, and a second set
  would be a second precedence order — so two sections loaded by two
  builders from two file lists remain two configurations, not a rename.

- **`benches/instructions.rs`: instruction counts under callgrind, for the
  five claims this crate makes about cost.** Wall clock is measured by
  `read_path.rs` and `engine.rs` and gates nothing, because a shared
  runner's variance is larger than the regressions worth catching;
  instructions are the same number on the same binary, so they can.
  Measured on rustc 1.97.1 / valgrind 3.24.0 / glibc 2.43 / x86_64: a read
  is **85 instructions**, a thousand reads **75,023** — with two extra RAM
  hits for the additional nine hundred and ninety-nine, so nothing is
  allocated per read — a one-document load **20,942**, `explain` on one key
  **52,523**, and a twenty-key reload **183,791**. Four of the five are
  bit-reproducible across runs and rebuilds. Anything whose count is not
  stable — the filesystem, the watcher, a remote store — is deliberately
  absent. `just instructions` runs it; it needs valgrind and a
  version-matched `iai-callgrind-runner`, and CONTRIBUTING.md covers both,
  including building valgrind without root.

- **`whole_document()`: read a document that has no section header.** The
  default layout is one file, several sections — every top-level key names
  one, which is what lets a `config.toml` hold `[db]` and `[server]` for two
  types that know nothing about each other. A file that is *only* one
  configuration has no use for that header, and a file this crate did not
  write may have none to give: a container image's
  `{"host": "0.0.0.0", "port": 8000}`, a chart's rendered values, a file
  another tool owns.

  The key keeps every other job it has — the environment prefix is still
  `{prefix}{KEY}_`, the cache entry and the diagnostics are still named
  after it — and it applies to every document the load reads, files,
  discovered files, profile variants and a remote store's document alike,
  because sources that disagreed about their own shape would be a
  configuration nobody could reason about. `LoadSpec::with_whole_document`
  is the same switch for the macro-free API.

  A configuration with nothing to call itself may now pass an empty key,
  and the environment layer is then the prefix alone: `APP_PORT` rather
  than `APP__PORT`.

  The book's [Document Shape](https://dynamic-config-rs.github.io/document-shape.html)
  chapter is the whole story, with the three neighbouring questions it turned out nothing answered in one place: a key
  the file has and the type does not name (ignored by the load, reported by
  `check`, refused by `deny_unknown_fields`), two files holding half a
  struct each (they merge; later files win), and a field no source supplies
  (`ErrorKind::Missing`, naming the field). Every answer has a test in
  `tests/document_shape.rs` and a runnable `document_shape` example.

### Changed

- **`ConfigCell::store_with` returns the `Arc<T>` it installed** rather than
  `()`. Migration: a `store_with(..)` in tail position of a `()` function
  needs a `;`.

- **`explain`'s alias row names the old path**, through a new
  `Contribution::aliased_from` field and in the rendered table:
  `alias db::timeout   in /etc/app.toml`. `Contribution` is
  `#[non_exhaustive]`, so the field is additive.

- **An aliased value carries the old path's own provenance**, so
  `source_of` names the file holding the old spelling even when the alias
  created the destination outright — where it used to answer *origin
  unknown*.

- **`::` in an ordinary key path is an error** naming the alias syntax,
  instead of a key with a colon in its name: `set_default`, `set_override`,
  `set_flag`, `bind_env` and an alias's new path all refuse it.

- **`watch::spawn` and `watch::spawn_with` hand their reload closure the
  path that triggered it**: `impl Fn(&Path) -> Result<Option<String>,
  Error>` where it was `impl Fn() -> ..`. That path is the only place the
  changed file is known — the debounce collapses a flurry into one reload
  and used to drop it — and it is what `ReloadReason::FileChanged` carries.
  Migration: `|| ..` becomes `|_| ..`. One path, not the set: a window can
  cover several files, and the path names what *triggered* the reload;
  `changed_paths(old, new)` still reports which keys moved. `Builder::watch`
  is unchanged.
- **The last-known-good cache's fingerprint is computed differently, and
  files written by earlier versions will not be recognised.** It used to
  hash figment's `Debug` rendering of the resolved tree, which put the
  identity of every cache file at the mercy of an upstream crate's
  formatting — and carried figment's numeric *widths* into the hash, so the
  same document assembled from different providers fingerprinted
  differently. It now hashes this crate's own `Value` tree, which has
  neither a provenance tag nor a width. An unrecognised fingerprint means
  "do not recover", which is the conservative direction: a start that would
  have reported "nothing moved" now reports that the values differ.

- **The refusal a redaction-dependent cache earns now names every way to
  fix it.** It said only that the generated `builder()` knows which
  fields are secret, which stopped being the whole truth when
  `Builder::secrets` landed — and was never true for a language binding.
  It now names the declaration, `.secrets([..])`, the Python spelling of
  that, and `CacheMode::Full`.
- **The refusal a bare document earns now names the fix.** "top-level key
  `host` is not a table" is only obvious to somebody who already knows this
  crate's layout; it now goes on to say to read the file with
  `.whole_document()`.

### Fixed

- **The failure counter no longer uses a method nightly has deprecated.**
  `fetch_update` was renamed to `try_update`, which does not exist at this
  crate's 1.71 floor — so the saturating increment is now the
  compare-exchange loop `fetch_update` was doing anyway. It was not
  cosmetic: CI's documentation job runs nightly with `RUSTFLAGS: -D
  warnings`, where a deprecation is an error, so the next clean runner would
  have failed to build.

- **`Remote::clear()` did not bump the generation fence**, so a fetch that
  was in flight when a caller cleared the slot put the document back when it
  landed. Configuration somebody explicitly dropped came back from a network
  round trip that started before they dropped it. `clear` now fences exactly
  as `set` and `set_async` do.
- **`changed_paths` and `Snapshot::diff` compared rendered values**, which
  reported a key as changed whenever two providers supplied the same number
  at different integer widths — an audit log crying wolf, and every
  `on_change` filter firing. The comparison is structural now, over the
  untagged `Value` tree `to_value` already hands out.
- **The concurrent-writer contract on `on_reload` is documented.**
  `ConfigCell::on_reload`, `ConfigCell::on_reload_scoped`,
  `Dynamic::on_reload` and `Dynamic::on_reload_scoped` now say what
  overlapping reloads guarantee — a consistent `(previous, current)` pair
  every call, and no defined order between calls — and point a hook that
  needs a total order at `generation()`. The behaviour has not changed;
  reloads are still not serialised, on purpose.

- **`clear_remote()` no longer ends a running watch.** Clearing the fetched
  document bumped the same counter a source replacement bumps, so every
  `RemoteSink` taken before the call went permanently stale: the store had
  not changed and the stream was still delivering, but the next push was
  refused for belonging to a source that had been "replaced" — which is how
  a watch loop is told to stop. The two events are counted apart now, one
  number for source identity and one for the document epoch, so a `clear`
  still discards a fetch that was in flight and leaves the watch alone.
- **A fetch that lands after its source was replaced no longer reports on
  the replacement.** The document was fenced and the status was not, so an
  old fetch's success marked the new store as fetched and healthy — one
  nothing had yet spoken to — and an old failure marked it as down. Both
  status updates now happen under the same generation check as the document
  commit, and inside the one lock that reads it rather than as a check
  followed by a write.

### Security

- **Two more roads a configured value could reach a diagnostic on.** The
  unit adapters quoted the text they failed to parse — a password pasted
  into a `#[serde(with = "duration")]` field came back as `` `hunter2` does
  not start with a number ``, and one that began with digits had its tail
  quoted as the unknown unit — and `save` refused a configuration that does
  not serialize to a table by `Debug`-printing it, which for a newtype over
  a token is the token, on the path that was about to write it to disk.
  Both now name what was expected and what shape arrived, never the value.
  The unit messages still list the units they accept, and the loader still
  names the key and the file.

- **A profile variant could leave the directory of the file it varied.**
  `profile_variant` reasoned about the whole path rather than the file
  name, and stripping `.age` off a name like `..age` leaves a trailing `.`
  component whose *file name*, to `Path`, is the directory above it. So
  `/etc/my.app/..age` with `APP_PROFILE=production` resolved to
  `/etc/my.production.app.age` — one level up, in a directory the caller
  never named, which is exactly the traversal `profile_is_safe` exists to
  prevent. Directories with a dot in the name (`/etc/my.app`, `/srv/conf.d`)
  are ordinary, so nothing unusual on the caller's part is needed to reach
  it. The naming now happens on the file name alone and is rejoined to the
  original path once, so the sibling rule is structural rather than
  something each branch has to remember. Found by the new `sections` fuzz
  target, which asserts the rule directly.

- **Reading by path no longer echoes the value it could not convert.**
  `Snapshot::get`, `Snapshot::extract` and the new `Value::get_as`
  deserialize a value already in hand, so they never went through the
  loader's translation and rendered figment's message verbatim — and
  figment renders a mismatch as ``invalid type: found string "hunter2",
  expected u16``. A password typed into a numeric field is exactly how
  that happens, and reading by path is where it happens most. All three
  now take the same stripping every other backend failure here takes: the
  path and the kind of thing that was there, never the value.
  `tests/security.rs` pins all three doors.

- **A syntax error no longer quotes the line it failed on.** `toml` renders a
  parse failure by echoing the offending line under a gutter, and an
  unterminated string is the typo somebody makes while pasting a password
  into a config file — so the quoted line *was* the password, in every
  diagnostic that printed one. The position and the reason survive; the
  quoted document does not. JSON and YAML render a single line and are
  unaffected. Pinned by `tests/security.rs`.

## [0.5.0] — 2026-08-12

### Added

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
- **A model holding an enum, a date or a `Decimal` could not be
  diffed.** `changed_paths` and `set_defaults` take an instance back
  apart, and neither `model_dump()` nor `dataclasses.asdict` unwraps an
  `Enum` — so the audit half of a reload raised a `TypeError` for any
  schema with one in it, Pydantic's included. Enums now convert to their
  value, and the stdlib types a configuration legitimately holds —
  `date`, `time`, `datetime`, `Path`, `Decimal`, `UUID`, the `ipaddress`
  family — convert to the one text form each of them parses back from.
- **A native TOML date reached Python as a one-key dict.** figment
  carries dates, times and datetimes under a private marker that serde
  reconstitutes on the Rust side and nothing reconstituted on the Python
  one, so a `date` field met a table and every schema refused it. The
  binding now hands over the text the file wrote, which is what a schema
  can parse.
- **`Snapshot::to_value`'s integers survived the crossing only up to
  `i64`.** The binding's export cast anything larger to `f64`, so a
  perfectly ordinary `u64` identifier came back rounded from
  `snapshot().to_dict()` while the installed model kept it exactly. The
  export tries `u64` before the float now.
- **`bind_env` could not see a `.env` file.** A binding names one
  variable exactly, and a deployment that writes that variable into a
  `.env` file rather than exporting it means the same thing by it — yet
  bindings read only the process environment, so the field got nothing.
  The prefixed `.env` layer cannot cover the case either: it recognises
  only names built from the prefix and the key, and it is skipped
  altogether when there is no prefix, which is the usual shape for a
  program binding `PORT` or `DATABASE_URL` by name. Bindings now fall
  back to the `.env` files, below the real environment — the order those
  two layers were already in. Recovery from the last-known-good cache
  resolves them the same way.

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

- `apply_remote` → `remote_sink()` + `RemoteSink::apply`: pushes carry the
  generation of the source their loop was wired against, and a replaced
  source's sink refuses. `Remote::install` is no longer public.

### Added

- The concurrency claims are model-checked: under `--cfg loom` the library
  swaps its sync primitives for loom's (`src/sync.rs`), and `just loom`
  runs the remote fence — fetch and push — and the async wake protocol
  through every interleaving, on the real code. The check-register-check
  dance now lives in one place, `Notify::poll_with`, which is what the
  model drives.

## [0.2.0] — 2026-08-11

### Breaking

- **The attribute declares; the builder configures.** `#[dynamic_config]`
  takes no arguments; the `Builder` carries the whole source surface (plus
  `validate`, `watch_with`, `prepare`, `reload`, `schema`, and per-type
  memory of the configuration `init` installed). Generated
  `load`/`init`/`start_watch`/`save*`/`schema` methods are gone; the
  attribute error for any argument is the migration map.
- `watch::spawn` / `spawn_with` take an owned `watch::Watched` instead of a
  `LoadSpec<'static>`.
- The cache moved to `Builder::cache(path, mode)`; the mode is always
  spelled out, and redaction-dependent modes are refused without the
  generated builder's secret knowledge.

### Added

- `explain(path)` / `Explanation` / `Contribution`: per-layer provenance
  tables, secrets pre-redacted in the generated method.
- `Snapshot` carries provenance; `Snapshot::source_of(path)` answers for the
  snapshot in hand.
- `strict_env` (`.strict_env()` on the builder, `with_strict_env` on
  `LoadSpec`): ambiguous environment spellings are refused with the
  variable named.
- `Builder<T>` and the generated `builder()`: runtime-chosen sources that
  load — or install — with the attribute's exact semantics; now with
  `discover`, `cache` (+ recovery), `watch` and async `load`/`init`.
- `changed_paths(old, new)` (audit, paths only); watcher reloads are
  `config_reload` tracing spans with outcome and duration.

### Changed

- `changes()` before `init()` is contract: the initial install is the
  handle's first change.

### Fixed

- `Snapshot::fmt` (and `Recovery` through it) printed resolved values,
  secrets included; both now show keys and shape only.
- The `strict_env` refusal does not echo the offending value.

## [0.1.0] — 2026-08-10

### Breaking

- `Recovery::Drift` carries `Option<Vec<String>>`: `Some` is the fingerprint
  (key paths, or one explanatory sentence when only values moved), `None`
  means the comparison itself was impossible.
- `start_watch()` while already watching → `Err(AlreadyExists)`; watchers
  keyed by `TypeId` (per generic instantiation); `watch::spawn`/`spawn_with`
  take a `TypeId`.
- `changes()` waiters wake before reload hooks run.
- Empty-env rule unified (trim-empty + `allow_empty_env` everywhere,
  bindings included).
- Cache files carry a format marker; 0.0.x files read via a temporary
  fallback.
- A remote fetch overtaken by `set_remote` is discarded.

### Added

- `on_reload_scoped`/`HookGuard`; panic-isolated hooks; `set_defaults`;
  deterministic alias chains + cycle rejection; fsync'd atomic writes;
  honest drift reports (including "could not compare").

### Fixed

- Non-UTF-8 environment no longer panics `load()`; recovery validates,
  seeds the diff baseline and keeps env above `.env`; renamed secrets
  redact correctly; path-shaped profiles refused; debounce bounded and
  pre-filtered; `Fetched` Debug redacted; zeroization covers the encrypted
  *write* path.

## [0.0.1] — 2026-08-10

Initial release.

### Added

- `#[dynamic_config]` — one attribute wiring loading, storage and reload:
  `files`, `name`+`paths` discovery, `key`, `env` (+`nest`,
  `allow_empty_env`), `watch` (+`debounce`, `poll`, `poll_interval`),
  `async`, `profile_env`, `validate`, `diff`, `env_files`, `save`, `schema`,
  `cache` (+`cache_mode`).
- Layered precedence with provenance: `source_of` names the file, variable or
  store a value came from; `check()` reports the whole configuration at once.
- Lock-free reads: `current()` is one atomic load. Reload callbacks,
  `changes()` for async waiters, and a `Group` for types that reload
  together.
- Runtime layers (`set_default` / `set_override`), key aliases, per-field
  environment bindings (`bind_env`), clap integration, `"30s"`/`"64MiB"`
  units.
- Remote stores behind `RemoteSource` / `AsyncRemoteSource`; fetching is
  explicit, so `load()` never touches the network.
- A last-known-good cache with three modes (full / redacted / fingerprint),
  written `0600` and atomically.
- Transparent decryption of `.age` config files, `save_encrypted`, and a
  `Decryptor`/`Encryptor` pair for schemes of your own; decrypted text is
  zeroized on every path.
- JSON Schema export (`schema()`), `.env` files as the environment layer,
  foreign figment providers as sources.
- Diagnostics report paths and types, never values — enforced by its own
  test suite.

[Unreleased]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.3...v0.7.0
[0.6.3]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/ctolon/dynamic-config/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/ctolon/dynamic-config/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

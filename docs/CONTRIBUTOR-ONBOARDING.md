# Contributor onboarding

A tour of every crate, every module and every feature — what it does, why it is
shaped that way, and where you would touch it to change something.

Read [README.md](../README.md) first if you have not: it is the specification,
and this document assumes you know what the crate does from the outside.
[CONTRIBUTING.md](../CONTRIBUTING.md) is the short version of the rules;
[AGENTS.md](../AGENTS.md) is the same ground for coding agents.

---

## 1. The shape of the workspace

Twelve crates, one version. Ten publish to crates.io in three waves, the
Python extension publishes a wheel to PyPI in a fourth, and the CLI rides
the third.

```text
dynamic-config-macros      the proc macro. No API of its own.
        ↓
dynamic-config             everything with behaviour.
        ↓
dynamic-config-etcd        one remote store each. Independent of one another.
dynamic-config-consul
dynamic-config-nats
dynamic-config-redis
dynamic-config-vault
dynamic-config-s3
dynamic-config-firestore

dynamic-config-embedded    a separate no_std crate. Shares no code with any of them.
```

**Why the macro is separate.** `#[proc_macro_attribute]` requires
`[lib] proc-macro = true`, and a crate with that set can export nothing else.
The same split `serde` and `serde_derive` have, for the same reason.

**Why the stores are separate crates.** Reaching for etcd should not put a
streaming client, two HTTP clients and an AWS SDK into a build that never asked
for them. The core keeps *no* network dependency — only the traits.

**Why `embedded` shares nothing.** figment is `std`. There is no common subset
to factor out, so trying would produce an abstraction that fits neither. It
keeps the *shape* and none of the code.

### Getting a working checkout

```sh
git clone https://github.com/ctolon/dynamic-config && cd dynamic-config
cargo install just cargo-release cargo-deny
rustup toolchain install 1.71 1.74 1.83 1.85 1.88   # the MSRV floors
rustup target add thumbv7em-none-eabihf          # for the no_std check
rustup toolchain install nightly                 # `just docs` builds like docs.rs

just check         # what CI runs
just containers    # the seven stores, against real servers; needs Docker
```

`just check` should be green on a fresh checkout. If it is not, that is a bug
and worth an issue before you change anything.

---

## 2. `dynamic-config` — the core

### 2.1 The path a value takes

Everything below is in service of one journey. Follow it once and the module
list stops being a list.

```text
#[dynamic_config]                  expand/ generates the type's storage, accessors
        ↓                          and `builder(key)` — the attribute takes no arguments
Builder                            builder/ — files, env, discovery, cache, validate,
        ↓                          chosen at runtime; funnels into one `LoadSpec`
LoadSpec                           source.rs — which sources, which section, which prefix
        ↓
loader::build()                    assembles figment providers in precedence order
        ↓
figment                            merges them
        ↓
loader::load::<T>()                deserializes the selected section
        ↓
ConfigCell::store()                cell.rs — an atomic swap, then hooks and wakers
        ↓
Config::current()                  an atomic load. No lock, no parse, no allocation.
```

### 2.2 Module by module

#### `lib.rs` — the front door

Crate documentation, re-exports, and `load()`. The hidden `__`-prefixed
`macro_rules!` the generated code calls live next door in `redirects/` —
`#[macro_export]` roots them at the crate top regardless of module, so they
could move out of the front page. Three remain, all item-level
(`__async_methods!`, `__async_remote_methods!`, `__clap_methods!`), for
methods whose *signatures* name a feature-gated type: a signature cannot
hide behind an expression-level `compile_error!`, and a `cfg` emitted into
generated code would be evaluated against the user's features instead of
ours. One family per file: `clap.rs` and `asynchronous.rs`. The expression-level redirects the attribute's source arguments once
needed are gone with the arguments — a format, `.env` file or encrypted
source whose feature is off is a *load-time* error naming the feature now,
because the path it arrives on is runtime data.

Still in `lib.rs` itself: the logging helpers `__log_remote_*` — those are
*functions*, reached by path, so unlike an exported macro they must stay
somewhere `pub`-reachable from the root.

#### `builder/` — where a configuration is stated

`Builder<T>`: the runtime half of the attribute split. `mod.rs` holds the
struct, the fluent surface (`file`, `discover`, `env` and its knobs,
`env_file`, `profile_env`, the cache, the `validate` hook) and the one
private `with_spec` funnel, so the builder cannot drift from the
`LoadSpec` semantics. `lifecycle.rs` is everything that commits —
`load`/`init`/`reload`/`prepare`, the async variants, and recovery from
the last-known-good cache (`recover`), in `init` rather than `load`
because `load` stays pure. `diagnostics.rs` answers without installing
(`explain`, `source_of`, `is_set`, `snapshot`, `check`, `schema`);
`watching.rs` starts the file watcher through this builder; and
`configured.rs` is `Configured<T>`, which remembers the builder at a
successful `init` — how the generated type-level diagnostics answer for
the running configuration.

#### `source.rs` — what to read

`Format` (JSON, TOML, YAML), `Source` (a file, an encrypted file, inline text,
or a foreign figment provider), and `LoadSpec` — the struct that names
everything a load needs.

`LoadSpec` is built with `with_*` methods rather than a struct literal, so a new
knob does not break every call site at once. That is why the generated code
chains builders.

`Source::format()` returns `Option<Format>`: a provider parses nothing.

#### `loader/` — the heart

The one module worth reading in full. `mod.rs` is the API surface and the
precedence order; `sections.rs` maps keys to sections and merges files,
`environment.rs` the env layers, `aliases_pass.rs` the alias gap-fill,
`recover.rs` the cache path, `origin.rs` the error translation. It:

- Assembles providers in precedence order (`build`).
- Maps top-level keys to sections (`Sections`) — reimplementing figment's
  `nested()` so that `$schema` can be exempt and a non-table key gets an error
  that names itself.
- Decrypts encrypted files (`merge_encrypted_file`).
- Merges `.env` files (`merge_env_files`).
- Fills gaps from aliases (`apply_aliases`) — queried *after* everything else,
  because that is the only point at which "nothing supplies this" can be
  answered.
- Translates figment errors (`convert`, `message`, `kind_of`) — and drops the
  offending *value*, which is where a secret in a numeric field would otherwise
  leak.
- Answers "where did this come from" (`origin_of`), by recognising each layer's
  metadata name.

**The precedence order lives in `loader/mod.rs` and nowhere else.** If you
add a layer, that is the file, and the position needs an argument in a
comment.

#### `cell.rs` — where a snapshot lives

`ConfigCell<T>`: a `OnceLock<ArcSwap<T>>` plus a hook list and an async notify.
`store` swaps atomically, tells the hooks, and bumps the generation. `load` is
one atomic load.

`get_or_init` settles the race between two threads installing the very first
snapshot — and `Arc::ptr_eq` afterwards is how an *initialisation* is told from
a *reload* without a second flag to keep in sync.

#### `registry.rs` — storage for generic types

A `Config<Postgres>` cannot have a `static` of its own, so generic types go
through a `TypeId`-keyed registry. Measured, not assumed: a static read is
17 ns, the registry 27 ns, so both paths exist and a non-generic type pays
nothing. `benches/read_path.rs` is the measurement.

#### `layer.rs` — the runtime layers

`Layer`: a path-to-value map behind a mutex, used by `set_default`,
`set_override`, `set_flag` and `bind_clap`. `insert_path` expands `pool.max` into
the nested shape figment wants; `check_path` rejects paths that name nothing.

#### `bindings.rs` — variables that are not yours to name

`EnvBindings`, behind `bind_env("port", "PORT")`. One provider *per binding*,
because figment attaches metadata per provider and naming the variable is the
useful half of a diagnostic.

#### `aliases.rs` — old paths after a rename

`Aliases`, behind `alias("pool.size", "pool.max_size")`. Fills a gap rather than
overriding. `known_keys()` is what stops the old path being reported as a typo.

#### `dotenv.rs` — a `.env` as the environment

Parses `KEY=value` and feeds the environment layer. Deliberately does *not* call
`setenv`: mutating the whole process's environment to configure one struct is a
side effect nobody asked for, and it is not thread-safe.

#### `decrypt.rs` and `age.rs` — encrypted files

`Decryptor` and `Encryptor` traits, `set_decryptor`, and `Plaintext` — a wrapper
that zeroizes on drop. `age.rs` is one implementation; the traits are why there
could be others (SOPS through `sops -d`, a KMS).

#### `write.rs` — writing back

`save`, `save_new`, `save_encrypted`. The security-critical part is
`create_and_fill`: `create_new` so a planted symlink is refused rather than
followed, and `mode(0o600)` at *creation* so there is no window in which secrets
are world-readable.

#### `cache.rs` — last known good

Three modes — `full`, `redacted`, `fingerprint` — because what lands on disk is
a trade-off with no single right answer. Recovery reads no files: the files are
what broke.

#### `watch/` — the file watcher

Watches *directories*, not files: editors and `mv`-based saves replace the
inode, which silently detaches a file-level watch. `mod.rs` is what a
watcher is pointed at (`Watched`) and how it detects changes
(`WatchMode`); `handle.rs` starts and stops one — the one-watcher-per-type
registry and its failure rollback; `debounce.rs` is the background loop
that waits out an editor's flurry; `relevance.rs` decides which events are
about our files — including `is_mount_marker`, which is what makes a
Kubernetes ConfigMap update visible: the kubelet swings a `..data` symlink
and the file's own path never receives an event.

#### `remote.rs` — the remote layer

`RemoteSource`, `AsyncRemoteSource`, `Remote` (the slot), and
`RemoteWatch`/`Watching` (stopping a blocking watch loop). Fetching is explicit;
`load()` merges from memory.

#### `asynchronous.rs` — awaiting a reload

`Changes<T>`: a hand-written `Future` over a generation counter and a waker
list — `std`, and nothing else, which is why smol and Embassy drive it.
`off_thread` puts a blocking load somewhere it is allowed to block.

#### `check.rs`, `snapshot.rs`, `discovery.rs`, `group.rs`, `units.rs`, `schema.rs`, `error.rs`, `log.rs`

`check()` reports what a load *would* do without doing it, including unknown
keys with transposition-aware suggestions. `Snapshot` is a resolved section with
`get`/`sub`/`diff`. `discovery` implements `name` + `paths`. `group` is
all-or-nothing multi-struct reload. `units` is the `"30s"`/`"64MiB"` serde
adapters. `schema` emits a JSON Schema of the *file*. `error` is the one error
type. `log` routes diagnostics to `tracing` or stderr.

### 2.3 The features, and what each buys

| Feature | Pulls in | Raises MSRV to |
|---|---|---|
| `json`, `toml`, `yaml` | a parser each | — |
| `watch` | `notify` | 1.85 |
| `async` | nothing at all | — |
| `tokio` | `tokio` (rt only) | — |
| `clap` | `clap` | — |
| `schema` | `schemars` | 1.74 |
| `decrypt` | `zeroize` | — |
| `age` | `age` | 1.85 |
| `dotenv` | nothing | — |
| `figment` | nothing (re-export) | — |
| `tracing` | `tracing` | — |

Three mandatory dependencies and no more: `figment`, `serde`, `arc-swap`.

**MSRV is measured, not declared.** `age` says 1.74 and needs 1.85, because its
translation machinery reaches `sha2 0.11`. Every floor has a CI row against a
real toolchain.

---

## 3. `dynamic-config-macros`

`lib.rs` is the entry point, and `expand/` generates the `impl` — `mod.rs`
orchestrates and assembles the final `quote!`; `accessors`, `diagnostics`,
`remote`, `schema` and `watch` each build their slice of the methods, spliced
back in a fixed order so the emitted tokens do not depend on the layout.

The attribute takes **no arguments**: it declares, the builder configures.
`args.rs` exists to *reject* anything between the parentheses — its error
message is a map from each old argument to the builder method that replaced
it, because the migration is mechanical and the message is where people meet
it. What gets generated is exactly the part a runtime value cannot provide:
the storage slots, the accessors over them, `builder(key)` seeded with the
type's statics, and the field-derived diagnostics (`#[config(secret)]`,
unknown-key detection).

**The generated code is deliberately thin.** Everything with behaviour lives in
`dynamic-config` as an ordinary function that can be linted, stepped through and
unit tested. Generated code can be none of those things.

The slot helpers in `expand/accessors.rs` handle the generic/non-generic
split in one place: a non-generic type gets a `static`, a generic one gets a
registry lookup.

A new knob almost never touches this crate any more — it goes on `Builder`
and `LoadSpec`, which is common enough to have its own guide:
[`.claude/skills/add-macro-argument/SKILL.md`](../.claude/skills/add-macro-argument/SKILL.md).

**The trap worth knowing now:** `where Self: SomeTrait` on an inherent method
does not work. rustc rejects an inherent method whose bound a concrete `Self`
does not meet, at the *definition*. That is why `schema()` lives on
`Builder`'s own generic `impl` (which can state `T: JsonSchema`) and `save`
is a free function over any `Serialize` value, rather than either being a
method every generated type gets.

---

## 4. The seven remote stores

Alike on purpose. Each: reads one key, watches the way its protocol allows,
authenticates the way its ecosystem does, can take a client you already have,
and is tested against a real server in a container.

| Crate | Trait | Watches by | Reads |
|---|---|---|---|
| etcd | async | a gRPC watch stream | a whole document |
| consul | blocking | a blocking query | a whole document |
| nats | async | a JetStream KV stream | a whole document |
| redis | blocking | keyspace notifications | a whole document |
| vault | blocking | polling the KV v2 version | a map of fields |
| s3 | async | polling the ETag | a whole document |
| firestore | blocking | polling `updateTime` | a map of fields |

Three rules every one of them follows, and a new one must:

- The current value is **not** delivered at startup.
- A deleted key is **not** a change.
- A transport failure **retries**; a failure from the caller's callback **stops**.

Adding one has its own guide:
[`.claude/skills/add-remote-store/SKILL.md`](../.claude/skills/add-remote-store/SKILL.md).

---

## 5. `dynamic-config-embedded`

`#![no_std]`, no allocator, no runtime. Four small modules: `lib.rs`
(`Format`, `Validate`), `cell.rs` (`ConfigCell` over a `critical-section`),
`asynchronous.rs` (`Changes` with four fixed waker slots), `error.rs` (a `Copy`
error with a `&'static str`).

**Why four waker slots** rather than a `Vec`: no allocator. Four is the shape of
the problem — a device has a handful of tasks that care about configuration. A
fifth waiter replaces the oldest, because a task never woken is a hang and one
woken early merely polls again.

**How it is tested.** On a host with the `std` feature, which supplies the
`critical-section` implementation a device gets from its HAL — the same code
otherwise. CI additionally *builds* for `thumbv7em-none-eabihf`, because a host
build cannot check `no_std`: `std` is in the sysroot and links itself in.

---

## 6. Testing

### Where a test belongs

| Kind | Where | Runs |
|---|---|---|
| One function's behaviour | `#[cfg(test)] mod tests` in the module | always |
| The crate's promises through its public API | `dynamic-config/tests/*.rs` | always |
| A diagnostic's exact wording | `tests/ui/*.rs` + `.stderr` | stable only |
| A diagnostic that needs a feature **off** | `tests/ui-no-decrypt/` | that build only |
| A store's real behaviour | `dynamic-config-*/tests/against_*.rs` | needs Docker |

### The mistake this repository keeps making

**Tests that share state.** A config type's snapshot, layers, aliases and
bindings live in `static`s keyed by the type. Two tests that share a config
type, a fixture path or an environment variable will race — and pass alone,
which is worse.

**One type, one fixture, one variable per test.** A `macro_rules!` to declare
them is normal here. This has been the cause of every intermittent failure the
repository has had — in `layers.rs`, `async_api.rs`, `integrations.rs`,
`safety.rs`, `remote.rs`, `write.rs`, `cache.rs`, `group.rs`, `env_bindings.rs`
and `dotenv.rs`.

Run both orders before believing a suite:

```sh
cargo test --workspace --features full
cargo test --workspace --features full -- --test-threads=1
```

### Container tests

Real servers, no mocks. A mock of etcd would only confirm what we already
believed about etcd — and several facts these tests pin are ones a mock would
have got wrong: etcd's client connects lazily, a NATS KV bucket must already
exist, Consul's first blocking query answers immediately, Redis publishes
nothing unless keyspace notifications are on.

---

## 7. Extending the crate

### A new format

`Format` in `source.rs`, a feature, and an arm in `merge_file`/`merge_text` —
with the feature off, the arm is compiled out and loading that format is a
load-time error naming the feature to add. Consider whether
`Source::provider` is the better answer — a format nobody else wants belongs
in the caller's crate.

### A new layer

`LoadSpec` gets a field and a `with_*`; `loader::build` gets a merge in the
right place, with a comment arguing for the position; `origin_of` gets a way to
recognise it; the README's precedence diagram gets a column.

### A new store

See the skill. Copy the closest existing crate.

### A new diagnostic

Anything a user reads is API. A new message goes in `tests/ui/` with `just
bless`, and must report paths and types rather than values.

---

## 8. What is load-bearing

Changing any of these is allowed. Arguing for it is the price.

- **Reading is lock-free.** `current()` is an atomic load and nothing more.
- **figment does not appear in a signature** unless the `figment` feature is on.
- **Secrets are paths and types, never values** — in diffs, reports,
  suggestions and error messages alike. `tests/security.rs` enforces it.
- **Files this crate writes are created private**, never chmodded afterwards.
- **A remote store is untrusted input.** Nothing it sends may panic the process.
- **No mandatory dependency** beyond `figment`, `serde`, `arc-swap`.
- **`#![forbid(unsafe_code)]`** everywhere, checked by CI.
- **The core MSRV is 1.71**, and every feature that raises it says so.

---

## 9. Sending a change

1. An issue first, for anything larger than a fix.
   [Not planned](../book/src/limitations.md#not-planned) records what was refused and what
   would reopen it — a refusal with a bad reason should be revisited, and that
   is worth saying rather than working around.
2. `just check` green, and `just containers` if you touched a store.
3. A test that would fail without your change.
4. Documentation if a user would notice; a `CHANGELOG.md` entry either way.
5. Open the pull request. The template asks for the decision you made, if there
   was one — that reasoning is the part a future reader cannot recover from the
   diff.

Do not run `cargo publish`. `cargo release` prepares and CI publishes on the
tag; see [RELEASING.md](../RELEASING.md).

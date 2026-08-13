# Schemaless Configuration

A struct is this crate's declaration. It is what makes `check()`'s
unknown-key detection, `#[config(secret)]` redaction and typed errors
possible at all — none of them are conveniences layered on top, they are
consequences of a type existing.

Some programs cannot write that type. A plugin host learns its plugins'
keys at runtime; a feature-flag table has a hundred short-lived keys and
no reason to name any of them in Rust; a tool that inspects somebody
else's configuration has no business declaring its shape. For those,
[`Value`] *is* the configuration type:

```rust
use dynamic_config::{Builder, Dynamic, Value};

let config = Dynamic::new(Builder::values("db").file("config.json"));
let values = config.init_and_current()?;          // Arc<Value>

let host = values.get("host").and_then(Value::as_str);
let max  = values.get("pool.max_size").and_then(Value::as_i64);
```

There is no feature to turn on and no dependency to add. `Value` is the
crate's own tree — the one `Snapshot::to_value` has always returned — and
the only thing 0.6 added to it is a `Deserialize` implementation, because
`DeserializeOwned` is the single bound the engine puts on a configuration
type. Everything else follows from that one line.

## What still works, which is nearly everything

Nothing in the engine ever knew what `T` was, so nothing in it changes:

| | Works |
|---|---|
| Files, discovery, profiles, `.env`, `secrets_dir`, the environment layer | yes, unchanged |
| Precedence, deep table merging | yes, unchanged |
| `watch()` / `watch_with()`, debounce, poll mode | yes, unchanged |
| The last-known-good cache and recovery | yes — see the secrets section for `Redacted` |
| `on_reload`, `on_reload_with`, `changes()`, `status()`, `ReloadGroup` | yes, unchanged |
| `source_of`, `is_set`, `explain`, `snapshot`, `Snapshot::sub` | yes, unchanged |
| `validate(..)` | yes — the closure receives `&Value` |
| `changed_paths(previous, current)` | yes — `Value` serializes, so a reload hook can diff two trees by path |

A reload replaces the whole tree, exactly as it replaces a whole struct.
An `Arc<Value>` taken before a reload keeps reading the generation it was
taken in; a key that appears for the first time is simply *there*, because
there is no schema to bar it.

One row is missing from that table and its absence is not about schemas:
**remote stores, `set_default` / `set_override` and `bind_env` live in a
`#[dynamic_config]` type's statics**, so no builder made with
`Builder::new` or `Builder::values` reaches them — a typed `Dynamic<Tenant>`
has exactly the same gap. A schemaless configuration that must read from a
store can still do it through `LoadSpec::with_remote` and `load::<Value>`,
without the install-and-watch machinery on top. See [Dynamic
Instances](dynamic-instances.md#where-the-type-surface-still-wins).

## What it does not get

Four things follow from a declaration, and nothing reconstructs them
without one. All four fail loudly rather than quietly, which is the part
that had to be designed rather than assumed:

**Types are checked at the read, not at the load.** A struct's load fails
on `port = "eight"`; a schemaless load succeeds and
`values.get_as::<u16>("port")` fails, at the moment it is read. Errors
name the path and the kind of thing that was there — never the value.

**Unknown keys are not checked, and `check()` says so.** With no field
list there is nothing to compare against, so `Report::unknown` is empty
for the same reason a blank page has no errors. `Report::unknown_checked`
is `false` and the rendered report carries a line saying
`unknown keys: not checked (no field list)` — an empty list that read as
an all-clear would be the worst outcome of this feature.

**Missing values are not missing.** A struct's required field is a load
failure; here, absent is `None` at the read.

**Secrets have to be named.** The next section, because it is the sharp
one.

## Secrets

`#[config(secret)]` is a *declaration*. A configuration with no struct has
nowhere to make one, so a schemaless configuration begins with no secret
list at all. The consequences are drawn deliberately:

**The tree never prints.** `Value`'s `Debug` shows shape and keys and
never values, exactly as `Snapshot`'s does — so `{:?}` in a log line, the
way resolved secrets usually escape, is closed whether or not anything was
declared. There is also deliberately **no `Display`**: a type that
rendered itself into `{}` would put a password wherever a program formats
a value it did not inspect. The ways out are all explicit — the
accessors, `get_as`, `render(Format)` for a document, and `Serialize` for
a serializer the caller chose.

**A redaction-dependent cache is refused, not guessed.**
`CacheMode::Redacted` and `CacheMode::Fingerprint` need to know which keys
to drop. With no list, `init()` fails with a message naming the problem
and **writes nothing** — the alternative, a file on disk marked "redacted"
with the password still in it, is the quiet worst case.

**`explain` prints values, and redacts against a list you supply.** It is
the one diagnostic here whose job is values; that is as true with a struct
as without one. `Builder::secrets(&[..])` is the declaration, moved to the
only place that knows it:

```rust
let config = Builder::values("db")
    .file("config.json")
    .secrets(&["credentials"])                     // paths, dotted
    .cache("last-known-good.json", CacheMode::Redacted);
```

The list buys exactly what the attribute buys: the same three-way rule
(the path *is* a secret, sits *under* one, or *contains* one), so naming
`credentials` redacts `credentials.password` too — and it makes the
redacted cache legal. What it cannot buy is a redacting `Debug`, because
there is no type to generate one for; `Value`'s own `Debug` already covers
that gap.

A schemaless configuration that loads secrets and supplies no list is
using `explain` on a plaintext value. That is the one thing here that
degrades silently, which is why it is written down twice.

## What a read costs

The crate's central claim is that reading configuration is an `ArcSwap`
load and a field access. A path read cannot be that — it walks — and the
honest thing to do is measure the difference rather than describe it.
`cargo bench -p dynamic-config --features json --bench read_path` prints
the machine above the numbers, because a nanosecond figure without one is
not a measurement:

```text
  cpu         Intel(R) Core(TM) i7-14700F
  cores       28
  target      x86_64-linux
  build       release
  rounds      5000000 per measurement
```

| | ns per read | against the field read |
|---|---|---|
| `Plain::current().port` — a struct field | 19.8 | — |
| `Dynamic<T>::current().port` — the same, instance-owned | 17.6 | 0.9× |
| `values.get("port")` — one segment | 27.2 | 1.4× |
| `values.get("pool.max_size")` — two segments | 32.1 | 1.6× |
| `values.get_as::<u16>("pool.max_size")` | 36.7 | 1.9× |

All six rows come from **one run** — comparing a number from a quiet
machine against one from a busy machine is how a benchmark lies — and that
run is the quietest of five on a machine that was doing other work. A
loaded run moves every row up by half again and leaves the ratios roughly
where they are.

Reading by path is **not free and not expensive**: the `ArcSwap` load is
unchanged and shared by both shapes, and what a path adds is a `split('.')`
and one `BTreeMap` lookup per segment — tens of nanoseconds, growing with
depth rather than with the size of the configuration.

`get_as` adds a rebuild of the value and a serde run on top of that walk.
The reason to prefer the accessors on a hot path is less the nanoseconds
than the `Result`: a conversion that can fail at every read is a
diagnostic-grade shape.

Allocations are the other half of the claim, counted rather than assumed
by `--bench alloc_profile`:

```text
static current()              0 allocations / 100000 reads
Dynamic current()             0 allocations / 100000 reads
Value::get(path)              0 allocations / 100000 reads
Value::get_as::<u16>          0 allocations / 100000 reads
Value::get_as::<String>  200000 allocations / 100000 reads
```

A path read borrows out of the installed tree, so it allocates nothing —
and neither does deserializing a scalar out of it. What allocates is
handing back something *owned*: two per `String`, which is inherent to
returning one rather than a property of this design.

The **ratios** travel between machines; the nanoseconds belong to the
block above them. Under a loaded machine every row roughly doubles and the
ratios stay put, which is why the argument is made with ratios.

## Why not a `DashMap`

It is the obvious reach for "configuration as a map", and it is the wrong
one *for reading*. Reads here are lock-free because the snapshot is
immutable and swapped whole: a sharded map would replace a pointer load
with a shard lock and buy nothing, because nothing mutates a single key.
The dependency would be added to make the read path slower.

Where a sharded map would genuinely pay is a different product — a runtime
registry with per-key writes, `set("feature.x", true)` mutating one key
without rebuilding the tree. That has a consistency story this design
deliberately refuses: a reader seeing key A's new value and key B's old one
is exactly what a whole-snapshot swap prevents. If that is the thing
wanted, it wants its own name.

## Why not behind a Cargo feature

The user request that opened this item suggested one, and the surface is
small enough to gate. It is not gated, for a reason worth stating: the
whole feature is a trait implementation on a type this crate already
exports. A `Deserialize` impl behind a `#[cfg]` means `Dynamic<Value>`
compiles in one build of `dynamic-config` and not another, and a library
depending on this one cannot rely on it at all — the same class of
invisible failure as a `cfg` in macro-generated code. There is no
dependency to save and no compile time worth the ambiguity.

## When to use which

Use a struct. It is the default for a reason: the compiler checks the
keys, the load fails at startup rather than at the read that needed the
value, secrets declare themselves, and `check()` catches the typo before
the deployment does.

Reach for `Builder::values` when the keys genuinely are not knowable at
compile time. Both shapes can coexist in one process — a typed
`ServerConfig` for what the program owns, a `Dynamic<Value>` per plugin for
what it hosts — and `Snapshot::sub(path)` hands a subsystem its own
sub-tree either way.

## In Python

The same idea, spelled `Values`:

```python
from dynamic_config import DynamicConfig, Values

config = DynamicConfig(Values, key="plugins").file("plugins.toml")
config.init()

config.current()["cache.ttl"]
```

It is a `Mapping` read by dotted path, and it gives up the same two
things this chapter's Rust half does — a field list for `check()` to
compare against, and a declaration of which paths are secret. See
[`Values`: a configuration with no
schema](python/types.md#values-a-configuration-with-no-schema).

[`Value`]: https://docs.rs/dynamic-config/latest/dynamic_config/enum.Value.html

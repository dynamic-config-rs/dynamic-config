# Dynamic Instances

`#[dynamic_config]` gives a *type* one configuration — the right default,
and a ceiling three kinds of program hit: multi-tenant services that want
one configuration per tenant, tests that want two side by side without
inventing marker types, and host-language bindings with no Rust type per
user class at all. [`Dynamic<T>`] is the same engine with the storage
owned by the value: its own snapshot, its own hooks, its own watcher
identity, nothing global.

```rust
use dynamic_config::{Builder, Dynamic};

let acme  = Dynamic::new(Builder::<Tenant>::new("tenant").file("acme.json"));
let umbra = Dynamic::new(Builder::<Tenant>::new("tenant").file("umbra.json"));

let a = acme.init_and_current()?;                     // Arc<Tenant>
umbra.init()?;
```

`init_and_current()` is `init()` with the snapshot it installed still in
hand. It is worth more here than on the type surface: an instance's
`current()` is an `Option`, so the split form ends in an `expect` whose
message only restates the line above it. What comes back is *that*
install's snapshot — a reload landing a moment later moves `current()` and
leaves this one alone, which is what "the configuration this program
started with" means.

The builder inside is the [same builder](builder-tour.md) with every
capability intact — files, discovery, the environment, profiles,
`validate`, the last-known-good cache. `Dynamic::new` only redirects where
a successful load *installs*: into a cell the instance owns instead of the
type's static.

## What changes, and what does not

**`current()` returns an `Option`.** The type-level `current()` panics
before `init()` with the type's name in the message; an instance has no
name to blame, so absence is an answer: `None` until the first successful
install. Everything else about reading is identical — one atomic load,
take the `Arc` once per request and reuse it.

**One watcher per *instance*.** The watcher registry keys types by
`TypeId`, which is meaningless for instances — every `Dynamic<Value>` is
the same type — so each instance carries a process-unique identity
instead. Two instances of one `T` watch side by side; a second `watch()`
on the *same* instance is `AlreadyExists`, exactly the
one-watcher-per-owner contract the type surface has, and dropping the
handle frees the slot the same way.

```rust
let _watch_a = acme.watch(Duration::from_millis(250))?;
let _watch_b = umbra.watch(Duration::from_millis(250))?;   // side by side
```

**Hooks, `status()` and `changes()` are per instance.** `on_reload` /
`on_reload_scoped` — and their event forms `on_reload_with` /
`on_reload_with_scoped`, which carry the
[reload reason](reload-lifecycle.md#why-a-reload-happened) — fire only for
the instance they were registered on;
[`status()`](reload-lifecycle.md#operating-a-configuration) counts only its
own installs and failures; and `changes()` is woken only by its own
installs — with the same
first-install contract as the type surface: a handle taken before `init()`
resolves on the first install, so it doubles as "wake me when this
configuration exists". The handle co-owns the storage, so it outliving the
`Dynamic` is safe rather than subtle.

**Diagnostics answer through the builder.** `source_of`, `is_set`,
`check`, `explain`, `snapshot` — the instance does not re-wrap them,
because the builder is where its sources live:

```rust
let origin = acme.builder().source_of("port")?;
let report = acme.builder().check()?;
```

**No `Clone`.** A `Dynamic` is an owner; share one behind an `Arc` when
several places read it. That keeps "who stops the watcher" a question with
one answer.

## The resolved tree as data

Some boundaries need the configuration as *values*, not as a type to
deserialize into — a language binding handing the tree to another
runtime's validator, an exporter. [`Snapshot::to_value`] walks the
resolved tree into an owned [`Value`] — seven shapes, no lifetimes, no
loader types in the signature, and never a JSON round trip:

```rust
use dynamic_config::Value;

let tree = acme.builder().snapshot()?.to_value();

assert_eq!(tree.get("port"), Some(&Value::Integer(5432)));
```

This is configuration handover, not a diagnostic: like `extract`, it
carries real values — secrets included. The paths-only rule governs what
this crate *prints*, not what it hands the program.

## Where the type surface still wins

A single global configuration read all over a program is what the
attribute is for: `AppConfig::current()` from anywhere, no value to
thread through call sites, compile-time knowledge of secrets for the
redacted cache, and the generated methods (`set_default`, `bind_env`,
remote stores) that live on type-level statics. Reach for `Dynamic` when
the configuration's identity is a *value* in your program — a tenant, a
test case, a foreign class — and for the attribute when it is the
program itself.

[`Dynamic<T>`]: https://docs.rs/dynamic-config/latest/dynamic_config/struct.Dynamic.html
[`Snapshot::to_value`]: https://docs.rs/dynamic-config/latest/dynamic_config/struct.Snapshot.html#method.to_value
[`Value`]: https://docs.rs/dynamic-config/latest/dynamic_config/enum.Value.html

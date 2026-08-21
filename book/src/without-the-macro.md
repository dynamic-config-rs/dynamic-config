# Without the macro

The engine is public and usable on its own:

```rust
use dynamic_config::{load, ConfigCell, Format, LoadSpec, Source};
use serde::Deserialize;

#[derive(Deserialize)]
struct Db { host: String }

static DB: ConfigCell<Db> = ConfigCell::new();

let sources = [Source::inline(r#"{"db": {"host": "localhost"}}"#, Format::Json)];
let db: Db = load(&LoadSpec::new("db", &sources))?;

DB.store(db);
assert_eq!(DB.load().unwrap().host, "localhost");
# Ok::<(), dynamic_config::Error>(())
```

## The bare builder

The same `Builder` the generated `builder(key)` returns can be built with no
config type at all — `Builder::new(key)` — for a load that wants the
builder's ergonomics without declaring anything:

```rust
use dynamic_config::Builder;

#[derive(Deserialize)]
struct Db { host: String }

let db: Db = Builder::new("db")
    .file(std::env::args().nth(1).expect("a config path"))
    .env("APP_")
    .strict_env()
    .load()?;
```

`load()` deserializes and installs nothing — on a bare builder there is
nowhere to install, so `init()`, `reload()`, `prepare()` and `watch()` all
refuse with an error saying to start from the generated `builder()`. The
bare builder funnels into the same `LoadSpec` as everything else, so
precedence, `strict_env`, `explain`, `source_of`, `snapshot` and `check`
mean exactly what they mean everywhere — with two honest gaps: `check()`
reports no unknown keys and a redaction-dependent cache mode is refused at
`init`, because only the generated `builder()` knows the field names and
which of them are secret.

On a `#[dynamic_config]` type, the generated `builder(key)` is this plus
somewhere to install: its `init()` feeds the same snapshot `current()`
reads, its `watch()` goes through the same one-watcher-per-type registry,
and each reload fires `on_reload` and wakes `changes()` like any other.
With the `async` feature, `load_async()` and `init_async()` do the same
off the executor. See the
[attribute reference](attribute-reference.md#the-builder) for the full
method list.

## LoadSpec, when even the builder is too much shape

`LoadSpec` is the struct every load funnels through — the macro-generated
surface and the builder alike. Assemble one by hand for an inline document,
a foreign provider, or a test that wants no filesystem at all; the
free functions `load`, `snapshot`, `source_of`, `is_set` and `explain` all
take one.

# Document Shape

Four questions about the deal between a document and a type, answered
here once for Rust and Python together. Every answer in this chapter has a
test behind it — `dynamic-config/tests/document_shape.rs` and
`dynamic-config-python/tests/test_document_shape.py` — and a runnable
example: [`document_shape`](examples.md) and
[`19_document_shape.py`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config-python/examples/19_document_shape.py).

| Question | Answer |
|---|---|
| Must a file be sectioned? | No. `whole_document()` reads `{"host": …, "port": …}` with nothing above it. |
| A key the file has and the type does not name? | Ignored by the load; `check()` names it. `deny_unknown_fields` / `extra="forbid"` / `forbid_unknown_fields` / a dataclass refuse it. |
| Two files, half the type in each? | One configuration. Later files win where they overlap. |
| A field no source supplies? | The load fails naming the field, unless a default or an `Option` covers it. |

## 1. A document with no section header

The default reading is **one file, several sections**: every top-level key
names one, and a configuration's key says which is yours.

```toml
[db]      # -> DatabaseConfig
host = "localhost"

[server]  # -> ServerConfig
port = 8080
```

That is what lets two configuration types that know nothing about each
other share a file, and it is why a top-level key that is not a table is a
parse error rather than an ignored line.

A file that is *only* your configuration has no use for the header — and a
file this crate did not write may not have one to give. A container
image's `server.json`, a chart's rendered values, a file another tool
owns:

```json
{ "host": "0.0.0.0", "port": 8000 }
```

Say so, and it reads:

```rust
let server: Server = Builder::new("server")
    .whole_document()
    .file("server.json")
    .env("APP_")
    .load()?;
```

```python
config = (
    DynamicConfig(Server, key="server")
    .whole_document()
    .file("server.json")
    .env("APP_")
)
config.init()
```

The Python decorator takes it as a keyword:

```python
@dynamic_config(key="server", files=["server.json"], whole_document=True)
class Server(BaseModel):
    host: str
    port: int
```

Without it, the same file is refused — and the refusal names the fix:

```text
top-level key `host` is not a table; every top-level key in a config file
is a section, so a value there must be a table (`$schema` is the one
exception). If this file is not sectioned — if the whole of it is one
configuration — read it with `.whole_document()`
```

### What the key still does

The key is not consumed by the document, so it keeps every other job it
has. Nothing else about the load changes:

| Still true with `whole_document()` |
|---|
| The environment layer is `{prefix}{KEY}_` — `APP_SERVER_PORT` reaches `port`. |
| Profile variants layer as usual: `server.production.json` over `server.json`. |
| `set_default`, `set_flag`, `set_override`, aliases, the secrets directory and `.env` files are unchanged. |
| The [last-known-good cache](persistence.md) and every diagnostic name the configuration after the key. |
| A [remote store](https://dynamic-config-rs.github.io/remote/)'s document is read the same way — headerless too. |

It applies to **every** document the load reads: sources
that disagreed about their own shape would be a configuration nobody could
reason about.

A configuration with nothing to call itself may pass an empty key. Then
the environment layer is just the prefix — `APP_PORT`, not `APP__PORT`:

```rust
Builder::new("").whole_document().file("server.json").env("APP_")
```

## 2. A key the file has and the type does not name

**The load ignores it.** A file may be shared with another configuration
type, with another tool, or with a later version of your own program;
refusing what one struct does not name would make all three impossible.

Ignored is not unnoticed. [`check()`](validation-diagnostics.md) compares
the section's top-level keys with the type's field names and reports what
it cannot place, with a guess when the key is close enough to be a typo:

```text
  hsot: unknown key, did you mean `host`?
  owner: unknown key
```

That is the answer to *why is my typo silently doing nothing* — and it is
a check you can run in CI without booting the program.

Three limits are worth knowing, and are the same ones
[`check`'s own documentation](validation-diagnostics.md) states. It needs a
**field list**, which is what the attribute supplies — through a
`#[dynamic_config]` type's generated `builder()` in Rust, and through the
model in Python; a bare `Builder::new(..)` knows the type only as `T`, and
its report says `unknown keys: not checked (no field list)` rather than
showing an empty list that reads as an all-clear. It compares **top-level**
keys only, because a proc-macro sees a field's type name rather than its
fields — `db.hsot` is caught, `db.pool.mx_size` is not. And it is skipped
entirely for a type with a `#[serde(flatten)]` field, which legitimately
absorbs keys the outer type never names.

If you want the strict reading, the *type* says so — the engine does not
second-guess it:

| Declaration | An unknown key is |
|---|---|
| Rust, by default | ignored |
| Rust, `#[serde(deny_unknown_fields)]` | a load error naming the key |
| Python, `BaseModel` by default | ignored |
| Python, `model_config = ConfigDict(extra="forbid")` | a load error naming the key |
| Python, `@dataclasses.dataclass` | **always** a load error naming the key |
| Python, `msgspec.Struct` by default | ignored |
| Python, `msgspec.Struct(forbid_unknown_fields=True)` | a load error naming the key |

The dataclass row is not an oversight. A dataclass has no `extra` setting
to consult, so there is nothing to choose from: the binding builds the
instance itself and refuses what the class does not declare, saying
`owner: Plain declares no such field`. If you need a dataclass to tolerate
extra keys, give it a field to absorb them, or use a Pydantic model or a
`msgspec.Struct` — both of which have a setting for it.

## 3. Two files, half the type in each

They merge. Nothing requires a single file to be complete:

```rust
Builder::new("server")
    .file("base.json")   // { "server": { "host": "0.0.0.0" } }
    .file("ports.json")  // { "server": { "port": 8000 } }
```

The rules, which are [precedence](sources-and-precedence.md) applied to
the ordinary case:

- **Call order is precedence.** Where two files set the same key, the
  later one wins.
- **Tables merge key by key**, so a three-line `secrets.json` can override
  two fields of a large `config.toml` without restating the rest.
- **Arrays are replaced whole**, never concatenated: there is no reading
  of `["a"] + ["b"]` that is right for every caller, and a silent append
  cannot be undone by a later file.
- **A file that is not there is skipped**, which is what makes an optional
  `secrets.json` work — and what makes a typo in a path quiet, so
  `check()` is where you find out which files actually contributed.

`whole_document()` changes none of this: two headerless documents layer
exactly as two sections do.

## 4. A field no source supplies

The load **fails**, and says which field:

```text
port: missing field `port`
```

In Rust the error carries it as data — `ErrorKind::Missing` and
`error.path() == "port"` — which is what lets a caller tell *you have not
configured this yet* from *the store is down*. In Python it is an
`InvalidError` naming the field.

Three ways to say a value is not required, in the order to reach for them:

| | Rust | Python |
|---|---|---|
| A constant fallback | `#[serde(default)]`, `#[serde(default = "…")]` | `port: int = 8000` |
| Absent is meaningful | `Option<T>` | `str \| None = None` |
| The program computes it | `set_default("pool.max_size", …)` | `config.set_default("pool.max_size", …)` |

The first two are the type's business and the engine has nothing to add.
The third is the layer *below* the files: for a value the program can work
out — cores × 4 — but a file should not have to state.

A **section no file mentions** is not a separate error. There is no
"unknown section" to report, because a section exists exactly where a
value does: what you get is the missing-field error above, for the first
field it looks for. The same is true of an empty document.

And a failing load still reports. `check()` answers when `load()` cannot —
that is why it is a separate call — so the failure is *in* the report
rather than in place of it, next to the paths that did resolve:

```text
[server]
  host                         in ./config.json

  would not load: port: missing field `port`
```

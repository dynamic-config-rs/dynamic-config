# Validation & Diagnostics

## `validate`

```rust
#[dynamic_config]
#[derive(Deserialize, Validate)]        // validator, garde, or a method of your own
struct Pool { min_size: u16, max_size: u16 }

Pool::builder("pool")
    .file("config.toml")
    .validate(|pool| dynamic_config::Error::ok_or_invalid(pool.validate()))
    .init()?;
```

`.validate(f)` runs after deserializing and before anything installs — on
`init`, on every watch reload, and on a recovery from the cache — so a reload
that fails validation keeps the previous snapshot exactly as a parse failure
does. For the case where every field is valid on its own and the whole is
still nonsense.

The check is a function you pass, not a method the macro resolves, so this
crate never pins a version of a validation library:
`Error::ok_or_invalid(..)` adapts whatever `Result` yours returns into
`ErrorKind::Invalid`.

## Key-level diffs

```rust
DbConfig::on_reload(|old, new| {
    for change in dynamic_config::changed_paths(old, new).unwrap_or_default() {
        tracing::info!(%change, "configuration changed");
    }
});
```

```text
pool.max_size changed
tls added
```

`changed_paths` reports which keys a reload changed. **Paths only, never
values** — otherwise a reload of `db.password` would do in the log exactly
what `#[config(secret)]` exists to prevent.

It runs in an `on_reload` hook, so it applies to every reload, not only the
watcher's: a document a
[remote watch](remote-stores.md#watching-a-store) pushed through its sink
is reported the same way. A program with no config file at all, watching only
a store, still learns what moved. For two resolved trees rather than two
structs, `Snapshot::diff` answers the same question.

## `#[config(secret)]`

```rust
#[dynamic_config]
#[derive(Deserialize)]          // note: no `Debug`
struct DatabaseConfig {
    host: String,
    #[config(secret)]
    password: String,
}
// DatabaseConfig { host: "localhost", password: "***" }
```

Generates a `Debug` that redacts the marked fields. `#[derive(Debug)]` alongside
it is a compile error rather than a race between two impls.

## Checking without booting

```text
$ myapp --check
[server]
  host                         set as command-line flag
  port                         from APP_SERVER_*
  tags                         in /etc/myapp/config.json

  hsot: unknown key, did you mean `host`?

  would not load: port: invalid type: found a string, expected u16
```

`check()` reports every key with the layer that supplied it, any key the struct
does not name, and why a load would fail. It **works when the load fails**,
which is the only time it is worth running.

**No values, ever.** A report that showed them would be pasted into an issue
tracker with the database password in it, undoing `#[config(secret)]`.

### What unknown-key detection catches

Top-level keys of the section, compared against the struct's field names —
`db.hsot` is caught, `db.pool.mx_size` is not. A proc-macro sees a field's
*type name*, not its fields, so nothing here knows what lives inside `pool`.

Suggestions use an alignment distance in which a transposition costs one edit,
because `prot` for `port` is how keys actually get mistyped; the threshold
scales with the name, so `id` tolerates one edit and `connection_timeout`
tolerates four.

Detection is skipped entirely when any field is `#[serde(flatten)]`: a flattened
field legitimately absorbs keys the outer struct never names, and reporting
those as typos would be worse than reporting nothing.

Where it is skipped — a flattened field, a bare `Builder::new`, a
[schemaless configuration](schemaless.md) — the report says so:
`Report::unknown_checked` is `false`, and the rendering carries
`unknown keys: not checked (no field list)`. An empty list and a list
nobody built mean opposite things, and only one of them is an all-clear.

## Where did this value come from?

```rust
DbConfig::source_of("port")?;   // Some(Origin::Env("APP_DB_PORT"))
DbConfig::is_set("pool.tls")?;  // false — absent, not "present but false"
```

Both re-read the sources, so they report what the *next* load would see rather
than what the current snapshot holds. The snapshot answers for *itself*:
`DbConfig::snapshot()?.source_of("port")` names the source of the value that
was actually resolved into that snapshot — provenance is captured while the
load still knows it. A snapshot that did not come from a live resolution (one
read back from the cache) has none.

## Why is this value what it is?

`source_of` names the winner. `explain` shows the whole argument:

```rust
println!("{}", DbConfig::explain("pool.max_size")?);
```

```text
pool.max_size = 32

layer        source                 value
default      set as default         8
file         in config.toml         16
environment  from APP_DB_POOL__MAX_SIZE  absent
override     set as override        32   ← winner
```

One row per layer that has anything to say, lowest precedence first; the
winner is the highest row with a value. Unlike every other diagnostic in this
crate, an explanation **contains values** — that is its point; you asked.
Fields marked `#[config(secret)]` come back with every value already `***`
(the origins stay — *where* a secret comes from is the useful half), and
[`Explanation::redacted`](https://docs.rs/dynamic-config/latest/dynamic_config/struct.Explanation.html)
blanks any explanation the caller knows to be sensitive — including a
secret's *old* key kept alive by an alias, which the field's marking cannot
cover. The rows are public on the returned `Explanation` for anything that
wants its own format.

The same two questions from a shell, without writing a program:

```sh
dynamic-config explain pool.max_size --file config.toml --key db --env APP_
dynamic-config diff old.toml new.toml --key db     # paths only, never values
```

That is `dynamic-config-cli`, an Experimental workspace member. A CLI cannot
see your attribute, so the flags restate the load — they have to match what
the application declares, or the answer is about a different load.

## Errors

One error type; `figment::Error` never reaches a signature, so a figment major
version bump is not automatically a breaking change here. Every error carries
the key path and the source that set the value:

```text
pool.max_size: invalid type: found a string, expected u16 (from APP_DB_)
```

**The offending value is not in the message.** The key, what kind of thing was
there, and the type that was wanted are all there — everything needed to fix it.
The value is not, because a password pasted into a numeric field would otherwise
land in a log line, and every other diagnostic here goes to some length to make
sure that cannot happen.

`Error::kind()` returns `Io`, `Parse`, `Missing`, `Type`, `Env`, `Invalid`,
`Remote`, `Decrypt` or `Backend`.

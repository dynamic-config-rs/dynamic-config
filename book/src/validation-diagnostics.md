# Validation & Diagnostics

## `validate`

```rust
#[dynamic_config(files = ["config.toml"], key = "pool", validate)]
#[derive(Deserialize, Validate)]        // validator, garde, or a method of your own
struct Pool { min_size: u16, max_size: u16 }
```

Every load calls `self.validate()` and turns an `Err` into `ErrorKind::Invalid`,
so a reload that fails validation keeps the previous snapshot exactly as a parse
failure does. For the case where every field is valid on its own and the whole
is still nonsense.

`validate` is resolved at **your** call site — an inherent method, or any trait
in scope — so this crate never pins a version of a validation library.

## `diff`

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, diff)]
```

```text
[dynamic-config] DbConfig: reloaded, pool.max_size changed, tls added
```

Logs which keys a reload changed. **Paths only, never values** — otherwise a
reload of `db.password` would do in the log exactly what `#[config(secret)]`
exists to prevent. Costs no extra file reads: the reload resolves once and both
deserializes and compares.

Applies to every reload, not only the watcher's: a document a
[remote watch](remote-stores.md#watching-a-store) pushed through `apply_remote`
is reported the same way. That is why it needs no `watch` — a program with no
config file at all, watching only a store, still wants to know what moved.

## `#[config(secret)]`

```rust
#[dynamic_config(files = ["config.toml"], key = "db")]
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

## Where did this value come from?

```rust
DbConfig::source_of("port")?;   // Some(Origin::Env("APP_DB_PORT"))
DbConfig::is_set("pool.tls")?;  // false — absent, not "present but false"
```

Both re-read the sources, so they report what the *next* load would see rather
than what the current snapshot holds.

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

# Patterns & Style

What using this well looks like, and the mistakes that are easy to make
because they *read* fine. Everything here is advice rather than a rule the
compiler holds — the rules the compiler holds are elsewhere.

## Declare one type per subsystem, not one per program

```rust
#[dynamic_config(key = "db", files = ["config.toml"], env = "APP_")]
struct Database { host: String, port: u16 }

#[dynamic_config(key = "cache", files = ["config.toml"], env = "APP_")]
struct Cache { url: String, ttl: Duration }
```

One `AppConfig` with everything under it works, and costs you the thing
this library is for: every subsystem reloads when *anything* changes, a
broken cache section takes the database's configuration down with it, and
the type that a function needs is bigger than the function. Sections are
cheap — they are the same file, read twice.

**The shape that tells you to split**: a struct whose fields belong to two
different teams.

## Read `current()` where you use it

```rust
fn handle(request: Request) -> Response {
    let db = Database::current();      // here
    connect(&db.host, db.port)
}
```

Not once at startup into a `static`, not passed down through six function
signatures. `current()` is an atomic load and an `Arc` clone; the cost of
reading it per request is smaller than the cost of the argument you would
thread through to avoid it — and a value captured at startup is a value
that has stopped reloading.

**Pass the `Arc` down only when a function must see one consistent
snapshot** across several reads. That is a real case, and it is the reason
`current()` hands back an `Arc` rather than a guard.

## Defaults belong in the type, not in the file

```rust
#[dynamic_config(key = "db", files = ["config.toml"])]
struct Database {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
}
```

A default in the file is a default every deployment copies and none of
them updates. A default in the type is the one place it lives, and
`set_default` is for the third case: a value the *program* computes, like
`num_cpus::get() * 4`.

## Validate what the type cannot say

`serde` gets you shapes and ranges through types. What it cannot say is
*this port must not be the one the metrics server uses* — and that is what
`validate` is for:

```rust
#[dynamic_config(key = "server", files = ["config.toml"], validate = check)]
struct Server { port: u16, metrics_port: u16 }

fn check(server: &Server) -> Result<(), String> {
    if server.port == server.metrics_port {
        return Err("port and metrics_port must differ".into());
    }

    Ok(())
}
```

A refusal here leaves the previous configuration serving, which is the
whole point: a bad edit is a failed reload rather than a broken process.

## Reload hooks do the smallest thing that works

```rust
Database::on_reload(|_previous, current| {
    pool.resize(current.pool.max_size);        // yes
});
```

A hook runs **inside** the reload, on whichever thread noticed the change
— often the watcher's. Blocking one blocks the reload; panicking in one is
caught, but the reload it interrupted is a failure. Anything slow belongs
behind a channel:

```rust
Database::on_reload(move |_previous, current| {
    let _ = sender.try_send(current.clone());   // and the work happens elsewhere
});
```

## Let the watcher own the reloading

```rust
let _watch = Database::watch(Duration::from_millis(250))?;
```

Keep the handle: dropping it stops the watcher, and a `let _ = ` drops it
immediately — the one mistake this API cannot prevent and the reason the
binding is named. Everything else — the debounce, the last-known-good
recovery, the refusal of a bad file — is already decided.

## Give the cache a path that survives a restart

```rust
.cache("/var/lib/myapp/last-known-good.json", CacheMode::Redacted)
```

The difference between "a broken config server means a warning in the log"
and "a broken config server means the fleet cannot start". `Redacted`
refuses to write unless the type says what is secret, which is the
feature rather than an obstacle.

## Say what is secret in the type

```rust
#[dynamic_config(key = "db", files = ["config.toml"], secrets = ["password"])]
struct Database { host: String, password: String }
```

One declaration drives three things: the cache drops it, `explain` renders
it `***`, and a validation failure keeps its location without its value.
Nothing else in the library has to be told.

## What to check in CI, and what to check at startup

| Question | Where |
|---|---|
| Does the committed config file still parse and validate? | CI, with `check()` or the CLI |
| Does *this deployment's* configuration load? | startup, and fail loudly |
| Is the store reachable? | a health endpoint, not a startup gate |

A startup that refuses to begin because a remote store is briefly
unreachable is a deployment that cannot roll out during an incident. That
is what the last-known-good cache is for.

## Names, in files and in variables

- **Sections are singular nouns**: `[db]`, `[cache]`, `[server]`.
- **Keys are what the field is called**, not what its type is:
  `max_size`, not `max_size_int`.
- **Environment variables inherit the prefix and the section**:
  `APP_DB_POOL__MAX_SIZE` is `db.pool.max_size` under `env("APP_")`.
  Nesting is `__` because a single `_` is a word separator in half the
  keys anybody writes.
- **A profile is an environment, not a customer**: `production`, `staging`
  — `config.production.toml`. One configuration per tenant is
  [dynamic instances](dynamic-instances.md), not a profile.

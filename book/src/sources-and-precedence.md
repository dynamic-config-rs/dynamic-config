# Sources & Precedence

## Precedence

```text
set_default < discovered < config.toml < secrets.json < remote < APP_DB_* < bind_env < set_flag < set_override
 (runtime)   (search path)   (first)      (last file)   (etcd…) (environment) (by name)  (CLI)     (runtime)
```

The two runtime layers bracket the rest:

```rust
DbConfig::set_default("pool.max_size", num_cpus::get() * 4)?;  // a computed fallback
DbConfig::set_override("host", "localhost")?;                  // a test, or --set
DbConfig::clear_overrides();
```

Defaults cover a fallback the program can compute but a file need not state —
`#[serde(default)]` handles the constant case, this handles the case where the
value is only known at run time. Overrides win over everything, which is what
makes them useful in tests and behind a `--set key=value` flag. Both take effect
on the next `load()`, and an error in either says `set as override` rather than
blaming a file.

`set_defaults` is the plural: it takes a whole `Serialize` value and makes
every field of it a default at once, which is how a hand-written
`Config::default()` becomes the bottom layer without naming each key.

Tables merge key by key, so a three-line `secrets.json` can override two fields
of a large `config.toml` without restating the rest. Arrays are replaced
wholesale, never concatenated — there is no reading of `["a"] + ["b"]` that is
right for every caller, and a silent append cannot be undone by a later file.

## Files

```rust
DbConfig::builder("db")
    .file("config.toml")
    .file("secrets.json")
    .init()?;
```

Sources merged **in call order** — later files win. The format comes from the
extension (`.json`, `.toml`, `.yaml`, `.yml`); using one whose feature is off
is a load-time error naming the feature to add. A file that does not exist is
skipped, which is what makes an optional `secrets.json` work.

Paths resolve against the working directory. For a deployment, prefer
[discovery](profiles-and-discovery.md#name--paths).

`.file(..)` and `.discover(..)` together is fine: the explicitly listed files
win, because a listed file is a deliberate statement and a search result is a
guess about the machine.

A `.age` suffix marks a file as [encrypted](encryption.md):
`secrets.json.age` is JSON that happens to be ciphertext.

A builder with no `.file(..)` calls at all says **no files, on purpose** — the
shape of a container whose configuration comes from a
[remote store](remote-stores.md) and the environment alone.

## `key`

```rust
DbConfig::builder("db")
```

The builder's one argument: the section this struct maps to. Every file's
**top-level** keys are sections, so several config types can share one file:

```toml
[db]      # -> DatabaseConfig
host = "localhost"

[server]  # -> ServerConfig
port = 8080
```

A consequence worth knowing: every top-level key must be a table. A stray
`"_comment": "..."` at the top level is a parse error, not an ignored key.

## `env`

```rust
DbConfig::builder("db").file("config.toml").env("APP_")
```

The prefix combines with the key, so `.env("APP_")` on a `"db"` builder reads
`APP_DB_*`. The
environment is merged after every file and wins over all of them.

| Variable | Sets |
|---|---|
| `APP_DB_HOST` | `host` |
| `APP_DB_MAX_SIZE` | `max_size` |
| `APP_DB_POOL__MAX_SIZE` | `pool.max_size` |

Values are read loosely: `8080` reaches a `u16`, `true` a `bool`, `[a, b, c]` a
`Vec<String>`. A value that cannot become the field's type is an error naming
the field.

## `nest`

```rust
DbConfig::builder("db").file("config.toml").env("APP_").nest("___")
```

The separator that introduces nesting in a variable name. Defaults to `__`.

A single separator cannot mean both "word break" and "nesting" — that is why the
default is doubled — so whatever this is set to must be something a field name
will not contain. Meaningful only alongside `.env(..)`.

## `allow_empty_env`

```rust
DbConfig::builder("db").file("config.toml").env("APP_").allow_empty_env()
```

By default `APP_DB_HOST=` counts as **unset** and the file's value survives. An
unset value rendered into a deployment template leaves exactly `FOO=`, and
letting that blank out a good configured value is a bad afternoon.

Turn this on when empty really is a value you need to be able to send.

## `strict_env`

```rust
DbConfig::builder("db").file("config.toml").env("APP_").strict_env()
```

figment reads environment values loosely — ergonomic, and ambiguous at the
edges. `APP_DB_TLS=off` reads like a boolean and arrives as the string
`"off"`: silently correct into a `String` field, silently wrong everywhere
else. With `strict_env`, the yes/no/on/off family (and `null`/`nil`/`none`)
is an error naming the variable — write `true`, `false`, or the value you
actually mean. `.env` files are held to the same standard. Loose stays the
default; strictness is a choice about your deployment's discipline, not ours.


## `env_file`

```rust
DbConfig::builder("db").file("config.toml").env("APP_").env_file(".env")
```

`.env` files, one call each, merged in call order just below the real
environment. Requires the `dotenv` feature and an `.env(prefix)` — a `.env`
holds variable names, and without a prefix there is no rule for which of them
belong to this section. See [`.env` files](#env-files).

## `.env` files

A `.env` holds *variable names*, not key paths, so it is not another format
for [`.file(..)`](#files) — it is the environment layer sourced from disk:

```rust
DbConfig::builder("db").file("config.toml").env("APP_").env_file(".env")
```

```text
APP_DB_HOST=localhost
APP_DB_POOL__MAX_SIZE=32
```

Same prefix stripping and same nesting as the real environment, merged just
below it — a variable somebody exported for this run beats a file in the
repository. A file that is not there is skipped, like any other.

**It does not touch the process environment.** `dotenvy` and friends call
`setenv`, which changes the environment of the whole program to configure one
struct: a side effect nobody asked for, visible to every library in the process,
and not thread-safe. This reads the file and merges it.

Variable interpolation (`${OTHER}`) and multi-line values are deliberately not
supported. Both are shell features that every `.env` library implements slightly
differently, and a configuration file whose meaning depends on which library
read it is worse than one that refuses.

## Variables that are not yours to name

The [`env`](#env) layer covers the case where the variable names follow from the
prefix, the key and the field. It does not cover the case where they do not:

```text
PORT                 the platform picked it — Heroku, Cloud Run, Fly
DATABASE_URL         a convention older than this program
REDIS_URL            an add-on wrote it into the environment
```

```rust
ServerConfig::bind_env("port", "PORT")?;
DbConfig::bind_env("url", "DATABASE_URL")?;
```

A binding sits just above the prefixed environment layer, because it is the more
specific statement: somebody named that variable on purpose, and the prefixed
one is a convention. It is read at **every** load, so a reload sees a change to
it, and a variable that is not set contributes nothing — which is the point,
since the platform may or may not have set it.

Nested paths work: `bind_env("pool.max_size", "DB_POOL_MAX")`. Binding the same
path twice replaces the first binding rather than layering it — two variables
for one field would have no defensible order between them.

## Command line

Flags sit above the environment and below overrides — a flag is typed by a
person for this one run, and should win over whatever the deployment happens to
export.

```rust
// One call per argument. `None` is a no-op, so unset flags leave the files
// alone and this is safe to run unconditionally.
DbConfig::set_flag("port", matches.get_one::<u16>("port").copied())?;

// Or hand clap the mapping and let it do the plumbing.
DbConfig::bind_clap(&matches, &[("db-host", "host"), ("db-port", "port")])?;

// Or the escape hatch, for keys with no flag of their own.
DbConfig::set_assignments(matches.get_many::<String>("set").into_iter().flatten())?;
```

Keys are relative to the section, so it is `host`, not `db.host`. Values are
read the way environment variables are, so `--set port=8080` and
`APP_DB_PORT=8080` mean the same thing.

`bind_clap` takes **only** arguments that came from the command line. clap's own
`default_value` is indistinguishable from a typed flag in `ArgMatches`, and
letting one outrank a configuration file would invert the whole precedence
order.

The `clap` feature is the only one that pins another crate's major version,
which is why it is separate and opt-in — everything above works without it.

## Old key paths after a rename

`#[serde(alias)]` covers a renamed *field*. It does not cover a renamed *path*:

```rust
DbConfig::alias("pool.size", "pool.max_size")?;
```

An alias **fills a gap rather than overriding** — a file that has been updated
wins over one that has not, whatever order they merge in, so a deployment
migrating one machine at a time gets no surprise.

The old key stops counting as an unknown key, because an alias that silenced
[typo detection](validation-diagnostics.md#what-unknown-key-detection-catches)
would make `pool.szie` a supported spelling. `source_of` reports the file
holding the old spelling rather than the alias, which is the more useful answer:
it names the file to edit.

## Bringing your own figment provider

With the `figment` feature, anything figment can read is a source:

```rust
use dynamic_config::figment::providers::{Format as _, Json};

// `.nested()` because this crate reads a top-level key as a section.
let provider = Json::string(document).nested();
let sources = [Source::provider(&provider)];
```

This is the **one** place figment appears in the API, which is why it is behind
a feature: with it off, a figment major bump is not a breaking change here; with
it on, you have opted into that coupling knowingly. figment itself is
re-exported so there is no second version in your graph.

Two things become yours to get right: the provider has to produce the section as
a profile (`.nested()` does that), and `source_of` reports its metadata name, so
a provider that describes itself badly produces a diagnostic that does too.

# Sources & Precedence

## Precedence

```text
set_default < discovered < config.toml < secrets.json < remote < secrets_dir < APP_DB_* < bind_env < set_flag < set_override
 (runtime)   (search path)   (first)      (last file)   (etcd…)   (a mount)   (environment) (by name)  (CLI)     (runtime)
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
win, because a listed file is a statement of intent and a search result is a
guess about the machine.

A `.age` suffix marks a file as [encrypted](encryption.md):
`secrets.json.age` is JSON that happens to be ciphertext.

A builder with no `.file(..)` calls at all says **no files, on purpose** — the
shape of a container whose configuration comes from a
[remote store](https://dynamic-config-rs.github.io/remote/) and the environment alone.

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

## `whole_document`

```rust
ServerConfig::builder("server").whole_document().file("server.json")
```

For a file that has no header to give — a container image's
`{"host": "0.0.0.0", "port": 8000}`, a chart's rendered values, a file
another tool owns. The document *is* the section, and the key goes on
naming the environment prefix, the cache entry and the diagnostics.

[Document Shape](document-shape.md) is the whole story, alongside the
three other questions people ask about the deal between a document and a
type: a key the type does not name, a field nothing supplies, and two
files holding half a struct each.

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

Variable interpolation (`${OTHER}`) and multi-line values are not
supported. Both are shell features that every `.env` library implements slightly
differently, and a configuration file whose meaning depends on which library
read it is worse than one that refuses.

## `secrets_dir`

Docker and Kubernetes hand a container its credentials as a *directory*: one
file per key, the filename is the key, the contents are the value.

```rust
DbConfig::builder("db").file("config.toml").secrets_dir("/run/secrets")
```

```text
/run/secrets/host                 → host
/run/secrets/pool__max_size       → pool.max_size
```

One directory level. Nesting is spelled in the filename with the same
separator [`nest`](#nest) sets, rather than with subdirectories: that is what
a Kubernetes secret actually produces, and it means one setting governs this
layer and the environment alike. A subdirectory is skipped, not descended
into.

The value is the file's contents with **one** trailing newline removed — every
tool that writes a secret to a file writes one, and nobody means it as part of
the password. A second newline is content and stays.

The layer sits above the files and the remote store and below `.env` and the
environment: a mounted secret is a fact about *this* deployment, so it beats a
document a central store hands to every deployment alike, and loses to a
variable exported for this one run.

A directory that is not there is skipped, exactly like a missing file — the
same image has to start in a test that mounts nothing. One that is there and
cannot be read is a load-time error naming the path, because that is a
permissions bug and silence about it would be worse. A file whose bytes are
not UTF-8 is an error too, rather than arriving lossily converted.

**Provenance is per file.** Each key is traced back to its own path, so
`source_of("password")` answers `/run/secrets/password` and `explain` names
the file rather than the directory — which is the useful answer when two
mounts disagree.

**Values arrive as strings**, always. The environment layer parses what it
reads, which turns an all-digit password into an integer and then fails the
`String` field it was meant for; a directory of credentials is the last place
that should happen. The cost is the other direction: a `u16` field cannot be
fed from a mounted file. Put numbers in the config file, where they read as
numbers.

Symlinks are followed but not descended into, which is what makes a real
Kubernetes mount work — every key there is a symlink into a timestamped
directory behind `..data`, and the two directories must not each contribute
the whole set again.

The directory is read on **every** load, so a rotated secret arrives with the
next reload however that reload is triggered. The
[file watcher](hot-reload.md) does not watch it, the same as for `.env`
files: it follows the configuration files and the discovery directories.

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
it, and a variable that is not set contributes nothing,
since the platform may or may not have set it.

Nested paths work: `bind_env("pool.max_size", "DB_POOL_MAX")`. Binding the same
path twice replaces the first binding rather than layering it — two variables
for one field would have no defensible order between them.

A binding also reads the `.env` files, below the real environment. A deployment
that writes `DATABASE_URL` into a `.env` file rather than exporting it means the
same thing by it, and the prefixed `.env` layer cannot serve that case: it
recognises only names built from the prefix and the key, and it is skipped
altogether when there is no prefix — which is the shape a program that binds by
name tends to have.

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

### A key that moved to another section

A field that left `[db]` and turned up in `[server]` is a rename a dotted path
cannot express, because each configuration type only ever sees its own section.
The old path may name the section it used to live in, with `::`:

```rust
// `timeout` used to be `[db] timeout`; it is `[server] timeout` now.
ServerConfig::alias("db::timeout", "timeout")?;
```

**The type that owns the key today declares where it used to live.** The other
direction — `DbConfig` announcing that one of its keys now belongs to
`[server]` — is a claim on somebody else's section that takes effect only if
that call happens to run before the other type loads; and in this very
migration the field has just been deleted from `DbConfig`, so there may be no
type left to make the claim. Declared on the destination, the alias is read
from the same place the load already looks, and reads at the call site as what
it is: this field used to be over there. Only the old path may be qualified.
`::` anywhere a path is otherwise accepted is an error rather than a key with a
colon in its name.

Everything else is the same word meaning the same thing. It **fills a gap**: a
`[server] timeout` that has been written beats a `[db] timeout` that has not
been deleted, whatever order the files merge in. It changes no typo detection —
`db` does not become a legitimate key of `[server]`, and `[db]`'s own `check()`
goes on reporting the key left behind as unknown, which is true and is how a
half-finished migration stays visible. `source_of` still names the file holding
the old spelling, and `explain` adds the other half of the answer, the spelling
itself:

```text
layer              source              value
file               /etc/app.toml       absent
alias db::timeout  /etc/app.toml       30   ← winner
```

Because the new path is never section-qualified, nothing can point *at* a
cross-section alias: it can only ever be the head of a chain, which bounds a
move between sections to one hop by construction rather than by a depth limit.

#### What it reaches, and what it does not

Every source this configuration lists is parsed whole and filed by top-level
key, so the other section is already in hand: no second file list, no second
read, nothing to cache. What is *not* in hand is the other section's
environment layer, defaults, flags or overrides — those are built from the key
this load is for, and a second set of them would be a second precedence order.
An alias reads the other section **as this configuration's own documents spell
it**.

That draws the boundary where it belongs: two sections loaded by two builders
from two different file lists are **two configurations, not a rename**. A
section that lives in a file this builder never reads resolves to nothing, and
the load carries on without it rather than failing. What the boundary buys is
that nothing goes stale — the file holding the old spelling is a file this
configuration already watches, so a reload sees it change.

The environment's old spelling has a better tool already:
`bind_env("APP_DB_TIMEOUT", "timeout")` names the variable exactly, whatever
section it was once built from.

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

Two things become yours to get right. The provider has to produce the section
as a profile (`.nested()` does that). And provenance comes from the metadata's
**source**, not its name: `Metadata::named("INI file")` alone leaves every
value it supplies answering `Origin::Unknown`, because `source_of` reads the
source while the name reaches only error messages. `Metadata::from("INI file",
path)` sets both, and a value then traces back to the file holding it exactly
as one from `.file(..)` does.

### A parser for a format this crate does not read

That is the whole plug-in point, and there is no second trait behind it: a
`Provider` turns text into `Map<Profile, Dict>`, and everything above it — the
layer order, provenance, the error type, reload — is unchanged. INI is the
worked example, because it is a format nothing here can read and thirty lines
of it settles whether the claim is true.

`dynamic-config/examples/ini_provider.rs` is the whole of it, and it runs:

```text
cargo run -p dynamic-config --example ini_provider --features figment,json
```

The parser is one method. An INI `[section]` header becomes a figment
**profile**, because a profile is what this crate reads a section from — that
single line is the mapping that makes the plug-in work at all:

```rust
impl Provider for Ini {
    fn metadata(&self) -> Metadata {
        Metadata::from("INI file", self.path.as_path())
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut sections: Map<Profile, Dict> = Map::new();
        let mut section = Profile::Default;

        for (index, line) in self.text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }

            if let Some(header) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
                section = Profile::from(header.trim());
            } else if let Some((key, value)) = line.split_once('=') {
                sections
                    .entry(section.clone())
                    .or_default()
                    .insert(key.trim().to_owned(), scalar(value.trim()));
            } else {
                return Err(figment::Error::from(format!(
                    "line {} is neither a section header nor `key = value`",
                    index + 1
                )));
            }
        }

        Ok(sections)
    }
}
```

Using it is the ordinary source list, and it is still later-wins:

```rust
let sources = [Source::inline(base, Format::Json), Source::provider(&ini)];
let database: Database = load(&LoadSpec::new("db", &sources))?;
```

Three things in that parser are decisions rather than mechanics, and each is
the kind that goes wrong quietly:

**A comment is a whole line, not everything after a `#`.** Trimming from the
first `#` would truncate a password at its first `#`, which is a character a
password is allowed to contain.

**The error names the position and the reason, never the line.** A line that is
not `key = value` is most often an unterminated quoted value — quoting it back
is how a pasted secret reaches a log. This is the same rule the built-in
loader follows, and `tests/security.rs` pins it there.

**Provenance comes from `Metadata`, and only from it.** A provider that
supplies `Metadata::named("INI file")` and no source leaves every value it
contributes at `origin unknown`; supplying the path makes `source_of("port")`
answer with that path. There is no `Origin` variant for "a parser plug-in"
and there is not going to be one — a second provenance vocabulary is exactly
what [refusing the trait](limitations.md) bought.

Types are yours too: INI has none, so the example parses `true`, then `i64`,
then `f64`, then falls back to a string, rather than letting `port = "5432"`
and `port = 5432` come to mean different things depending on which layer
supplied them.

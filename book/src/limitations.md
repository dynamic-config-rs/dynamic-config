# Limitations & Not Planned

## Limitations

- **Every top-level key in a config file must be a table**, with one exception:
  `$schema`, so a JSON file can point at the schema that describes it. Sections
  are figment profiles and a profile has to be a map, so a stray
  `"_comment": "..."` at the top level is an error — one that now names the key
  and says why.
- TOML datetimes are not modelled and deserialize as a table.
- The macro refers to the crate as `::dynamic_config`, so renaming the
  dependency is not supported.
- Error messages name the environment *prefix* rather than the exact variable,
  because that is the granularity figment reports.

## Not planned

Each of these is a real request with a real answer. They are refused rather than
unbuilt, so that nobody spends an afternoon discovering the reason — and each
says what would reopen it.

### A `ReloadExecutor` / `ReloadPolicy` abstraction

Exists under another name: `set_blocking_executor` already steers where the
blocking half of a reload runs, and the `tokio` feature installs the blocking
pool. A second abstraction over the same choice would be a synonym. Reopened
by a steering need the executor hook cannot express.

### `on_reload_async`

`changes()` *is* the async reload event — a `Future` on your executor, free
to await whatever the reaction needs. See
[The Reload Lifecycle](reload-lifecycle.md). Reopened by nothing; this is
what `changes()` is for.

### Splitting the core into `-core`/`-watch`/`-schema` crates

Feature flags already give the isolation a split would: disable a feature
and the dependency is gone. A crate split adds a version matrix to maintain
without removing anything anyone is forced to take. Reopened by a feature
whose dependency cannot be made optional.

### Fewer official store backends

The seven exist, are tested against real servers, and are marked
Experimental. Withdrawing shipped crates punishes their users to save
unshipped maintenance. Revisited per crate if one's client dependency
becomes unmaintainable.

### Nested profiles from figment

figment's profiles are a general mechanism. This crate spends them on
**sections** — `builder("db")` selects the `db` profile — and re-implements the
profile *idea* on top with [`profile_env`](profiles-and-discovery.md#profile_env) and sibling files
(`config.production.toml`). So a provider handed to
[`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider) cannot carry its own
profiles through.

The difficulty is not any one part; it is that `select(key)`, the section
mapping, `profile_env`, sibling files, `check()`, `source_of` and every
diagnostic that names a section all assume the current arrangement. Changing it
means giving sections a different mechanism and rewriting the layering
underneath everything that reads well today.

**What would reopen it:** a figment provider whose own profiles you need, where
`Source::provider` plus `profile_env` genuinely cannot express what you are
after.

### A swappable loader backend

"Make figment a plug-in" reads like extracting a `Loader` trait. It is not,
and the reason is worth writing down once rather than re-deriving.

figment reaches nineteen modules here, and not as a parser. `Snapshot` holds
figment's tree. Sections *are* figment profiles. Provenance *is* figment
metadata, converted at the one moment the figment that knew is still alive —
which is what `explain`, `check` and `source_of` read to say which layer
answered. Every layer, `.env` and the environment included, is a figment
provider in one table. A trait extracted across that seam would either carry
`Metadata` and `Profile` in its own signature — leaking exactly what the
[`figment` feature](stability-tiers.md#the-figment-feature-is-a-coupling-on-purpose)
exists to keep optional — or drop provenance, which is most of what this crate
sells.

The two things people actually want from it already exist and are narrower:

- **A format this crate does not read** is
  [`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider):
  three methods, and it has carried the section and provenance contracts since
  0.2.
- **Documents to combine before the loader sees them** — a store crate reading
  a prefix, a tool folding a fragment directory into one file — is
  `Value::parse` / `merge` / `overlapping_paths` / `render`, described in
  [Writing a Store](remote-stores/writing-a-store.md#several-keys-as-one-document).
  It hands over the parsing this crate already compiles, so nothing outside has
  to take `serde_json`, `toml` and `serde_yaml` as direct dependencies to
  re-do it.

**What would reopen it:** a backend that resolves layered providers, profile
selection and loose environment typing *and* carries provenance — at which
point the argument is about which backend, not about whether there is a seam.

### Case-insensitive keys

Viper lowercases everything. It hides typos — `Prot` and `port` become the same
key, so [unknown-key detection](validation-diagnostics.md#what-unknown-key-detection-catches) can never
tell you about the first — and it cannot round-trip: a configuration read and
written back comes out in different case from the one a person wrote.

**What would reopen it:** nothing. This one is a principle rather than a cost.

### HCL, Java properties, INI

Each is a parser and a set of edge cases for a format nobody here has asked for,
and none of them is something figment provides.

**The answer that is not a fork:**
[`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider)
takes any figment provider, so a crate that parses one of these wires in without
this one growing a dependency.

### Independent instances

Viper needs them because its default instance is a global. Here every
configuration type already has its own storage, keyed by the type — the same
isolation without the bookkeeping.

### Inferring a type from a default value

serde already knows the type. Viper's `SetTypeByDefaultValue` exists because
Go's `map[string]interface{}` does not.

### A service-account JSON key for Firestore

Signing one means an RS256 stack inside a configuration library, and Google's
own guidance is that a downloaded key is the option of last resort.
[Workload identity](https://github.com/ctolon/dynamic-config/blob/main/dynamic-config-firestore/README.md#authenticating) covers GKE, Cloud
Run, GCE and Cloud Functions; anything else can mint a token outside the process
and pass it in.

## Roadmap

[ROADMAP.md](https://github.com/ctolon/dynamic-config/blob/main/ROADMAP.md) is what might still be built, and why each item is not
obvious. It is short on purpose.

# Limitations & Not Planned

> Several refusals below are stated against Go's Viper, because that is
> where the behaviour being refused is best known. What this project took
> *from* it is in
> [CREDITS.md](https://github.com/dynamic-config-rs/dynamic-config/blob/main/CREDITS.md).

## Limitations

- **Every top-level key in a config file must be a table**, with one exception:
  `$schema`, so a JSON file can point at the schema that describes it. Every
  top-level key names a section, and a section is a table of settings, so a
  stray `"_comment": "..."` at the top level is an error — one that names the
  key and says why.
- TOML datetimes are not modelled and deserialize as a table.
- The macro refers to the crate as `::dynamic_config`, so renaming the
  dependency is not supported.
- Error messages name the environment *prefix* — `APP_DB_*` — when the exact
  variable cannot be confirmed. The variable is derived from the key path and
  used when it really exists; an aliased value, whose variable spells the old
  name, falls back to the prefix rather than naming a variable nobody set.

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

The eight exist, are tested against real servers, and are Beta as of
0.6.1. Withdrawing shipped crates punishes their users to save unshipped
maintenance. Revisited per crate if one's client dependency becomes
unmaintainable.

### Nested profiles from a foreign provider

A section here is the subtree under its key, and the profile *idea* is
implemented on top with [`profile_env`](profiles-and-discovery.md#profile_env)
and sibling files (`config.production.toml`). A figment provider handed to
[`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider)
files its values under figment profiles, and only three of them are read: the
default profile, the one named after the section being loaded, and `global`.
A provider whose own profile *hierarchy* is the point cannot carry it
through.

**What would reopen it:** a provider whose own profiles you need, where
`Source::provider` plus `profile_env` genuinely cannot express what you are
after.

### A swappable loader backend

**This one was reopened and then closed by building it.** Through 0.8 the
answer here was that figment reached nineteen modules, that sections *were*
figment profiles, and that provenance *was* figment metadata — so a `Loader`
trait would either leak `Metadata` and `Profile` into its own signature or
drop provenance, which is most of what this crate sells. The stated
reopening condition was "a backend that resolves layered providers, profile
selection and loose environment typing *and* carries provenance".

The engine is now that. It parses the documents, walks the environment,
reads the value strings, folds the layers and records the origin of each
leaf as it wins — [How resolution works](how-resolution-works.md) is the
whole of it — and figment is an optional interop adapter that is not in the
default dependency graph.

What is still refused is the *general* trait: one `Loader` interface with
several interchangeable implementations behind it. Two resolvers that agree
on nearly everything are two sets of edge cases, and the compatibility
contract's §5 says features are additive — a resolver behind a cargo feature
would make the meaning of a configuration depend on which features its
dependents happened to turn on. There is one semantics here, and it is this
crate's.

What people actually want from a seam exists and is narrower:

- **A format this crate does not read** is
  [`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider):
  three methods, and it has carried the section and provenance contracts
  since 0.2.
- **Documents to combine before the loader sees them** — a store crate
  reading a prefix, a tool folding a fragment directory into one file — is
  `Value::parse` / `merge` / `overlapping_paths` / `render`, described in
  [Writing a Store](https://dynamic-config-rs.github.io/remote/remote-stores/writing-a-store.html#several-keys-as-one-document).
  It hands over the parsing this crate already compiles, so nothing outside
  has to take `serde_json`, `toml` and `serde_yaml` as direct dependencies to
  re-do it.

**What would reopen it:** nothing about the backend. A *source* this crate
cannot express is a `Source` variant or a provider, not a second resolver.

### A crate per backend

`dynamic-config-engine-figment`, `dynamic-config-engine-config-rs` — one
crate each, so a build takes only the adapter it wants. It already takes
only what it asks for: a feature is an optional dependency, and a build
that does not name `figment` compiles no figment at all.
`tests/documents.rs` asserts that in the manifest so it stays true.

What a crate split would cost, on top of buying nothing measurable:

- **An adapter cannot be the default.** It has to depend on this crate for
  the `Engine` and `Reader` traits, so this crate cannot depend on it. The
  default engine is `config-rs` — and since this crate has no fold of its
  own, out of tree there would be no default at all.
- **The agreement tests would lose their list.** `engine::all()` is walked
  by the property test that holds every engine to the same fold, leaf by
  leaf. It can only list what is in the crate — so an out-of-tree adapter
  is one nothing compares, and a test that quietly stops testing is worse
  than one that was never written.
- **figment cannot leave anyway.** It is a permanent dev-dependency,
  because the ported value-string reader, deserializer, serializer and
  fold are each proved against it — and those comparisons reach private
  module internals that no other crate can see. Moving half of figment out
  would scatter it across a crate boundary, which is the opposite of why
  the backends were grouped into a directory each.
- **`Source::provider` would have to go.** It takes a
  `&dyn figment::Provider` and is part of this crate's public API.

**What would reopen it:** a backend heavy enough that carrying it as an
optional dependency costs something a feature flag cannot avoid — a build
script, a system library, a licence. None of the two is.

### Case-insensitive keys

Viper lowercases everything. It hides typos — `Prot` and `port` become the same
key, so [unknown-key detection](validation-diagnostics.md#what-unknown-key-detection-catches) can never
tell you about the first — and it cannot round-trip: a configuration read and
written back comes out in different case from the one a person wrote.

**What would reopen it:** nothing. This one is a principle rather than a cost.

### HCL, and the next format after it

INI and `.properties` are here, behind their own features, because deployments
asked for them. HCL is a parser and a set of edge cases for a format nobody
here has asked for, and the same is true of whatever comes after it: a format
in the crate is a format the crate maintains forever.

**The answer that is not a fork:**
[`Source::provider`](sources-and-precedence.md#bringing-your-own-figment-provider)
takes any figment provider, so a crate that parses one of these wires in
without this one growing a dependency.

### Independent instances

Example Go Viper needs them because its default instance is a global. Here every
configuration type already has its own storage, keyed by the type — the same
isolation without the bookkeeping.

### Inferring a type from a default value

serde already knows the type. Viper's `SetTypeByDefaultValue` exists because
Go's `map[string]interface{}` does not.

### A service-account JSON key for Firestore

Signing one means an RS256 stack inside a configuration library, and Google's
own guidance is that a downloaded key is the option of last resort.
[Workload identity](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config-firestore/README.md#authenticating) covers GKE, Cloud
Run, GCE and Cloud Functions; anything else can mint a token outside the process
and pass it in.

## Roadmap

[ROADMAP.md](https://github.com/dynamic-config-rs/dynamic-config/blob/main/ROADMAP.md) is what might still be built, and why each item is not
obvious. It is short on purpose.

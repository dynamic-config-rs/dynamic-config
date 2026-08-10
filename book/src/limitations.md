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

### Nested profiles from figment

figment's profiles are a general mechanism. This crate spends them on
**sections** — `key = "db"` selects the `db` profile — and re-implements the
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

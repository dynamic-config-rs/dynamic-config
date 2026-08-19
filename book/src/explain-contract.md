# Explain as a Contract

`explain(path)` answers the only question that matters at 3am: *why is
this value what it is?* Its answer is a stable type, not a log line —
which makes it something a CLI, a debug endpoint, or a support script
can build on. What follows is the shape that will not change under the
[compatibility contract](compatibility.md).

## The shape

An `Explanation` is **every configured layer's row for one path,
lowest precedence first** — absent layers included, because "the env
layer supplied nothing" is half of most surprises:

```text
db.pool = 32

  layer            source                          value
  defaults         set_defaults                    8
  file             /etc/app/config.toml            16
  env              APP_DB__POOL                    32   ← winner
  flags            —                               (nothing)
  override         —                               (nothing)
```

Each row is a `Contribution { layer, origin, value, aliased_from }`;
`winner()` is the highest row that supplies anything; the rows slice
is public for anything that formats its own table.

## The guarantees

- **Row order is precedence order.** The table *is* the
  [precedence documentation](sources-and-precedence.md), generated from the same
  code that merges — it cannot drift from reality.
- **Absent layers appear by name.** A layer you configured that
  supplies nothing shows as such; a layer you never configured is not
  invented.
- **Aliases say both halves**: the row is labelled
  `alias db::timeout` (the old spelling) and the origin names the
  file. Where it came from, and under which key.
- **`Display` shows values; `Debug` never does.** A routine
  `debug!(?explanation)` cannot leak — the value field prints `...`.
  `redacted()` blanks values while keeping origins: *where* a secret
  comes from is the useful half, and the safe one.
- **Values render short**: tables and lists show their shape
  (`a table (3 keys)`), not their contents.

## The three surfaces

| Surface | Call |
|---|---|
| Rust | `AppConfig::explain("db.pool")?` / free `explain(&spec, path)` |
| CLI | `dynamic-config explain db.pool` |
| Python / Node | `explain("db.pool")` — same rows, same order |

The bindings return the same rows in language-native shapes; the
[conformance suite](https://github.com/dynamic-config-rs/dynamic-config/tree/main/conformance)
holds provenance parity across all three.

## What it is for

Wire it to a `/debug/config` endpoint (redacted), a support-bundle
dump, or a startup `--explain <path>` flag. The type is stable so
those integrations survive engine upgrades — that is the contract in
the page title.

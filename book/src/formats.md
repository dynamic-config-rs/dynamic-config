# Formats

Five formats, each behind a cargo feature, each inferred from the file's
extension at load time. A file whose feature is off is a load-time error
naming the feature to enable.

| Format | Feature | Extensions | Types | Writable |
|---|---|---|---|---|
| JSON | `json` (default) | `.json` | its own | yes |
| TOML | `toml` | `.toml` | its own | yes |
| YAML | `yaml` | `.yaml`, `.yml` | its own | yes |
| INI | `ini` | `.ini` | widened | no |
| properties | `properties` | `.properties` | widened | no |

The first three are figment's providers. The last two are parsers in this
crate, added for the configuration that already exists in the world —
Java services carry `.properties`, and a generation of tools wrote
`.ini` — with **no new dependency and no effect on the MSRV**.

A `.age` suffix is looked through for all five: `config.ini.age` is INI
that happens to be encrypted.

## The INI dialect

"INI" names a family, so this crate's member is spelled out:

- `[section]` opens a table, and `[a.b]` opens a nested one — the
  git-config convention. Keys before any header sit at the root.
- Whole-line comments start with `;` or `#`. There are **no trailing
  comments**: a `#` inside a value belongs to the value, because a
  trailing-comment rule corrupts any value that legitimately carries one.
- No line continuations.

```ini
; the same document the TOML tour uses
[db]
host = db.internal
port = 5432

[db.pool]
max = 8
```

## The properties dialect

`java.util.Properties`, with the deviations stated:

- **UTF-8, not ISO-8859-1.** Modern JDKs read UTF-8 properties too; an
  escape-only encoding is a legacy this crate does not inherit.
- Dotted keys nest: `db.pool.max = 8` is the document
  `{db: {pool: {max: 8}}}`. `.` is to properties what `__` is to the
  environment layer.
- `=` and `:` both separate; the first unescaped one wins. A `\` ending a
  line continues onto the next, the continuation's leading whitespace
  trimmed. `\t` `\n` `\r` `\\` `\uXXXX` and escaped separators are
  honoured. Comments start with `#` or `!`.
- **A collision is an error, not last-wins.** `a = 1` and `a.b = 2` in
  one document contradict each other, and the error names both keys —
  and only the keys.

```properties
db.host = db.internal
db.port = 5432
db.pool.max = 8
```

## Where the types come from

Neither flat format has types, so values widen by the same rule the
environment layer applies: `true`/`false`, then integer, then float,
then string — and in INI a double-quoted value is a string, verbatim.
Your model still has the last word: widening feeds validation, it does
not replace it, and `port = "not a number"` fails exactly as it would
from any other source.

## Why neither can be written

[`save`](https://docs.rs/dynamic-config/latest/dynamic_config/fn.save.html)'s
contract is that what comes out can be read straight back in as the same
document. A format that widens strings on the way in cannot keep it —
`port = 8080` written today reads back as an integer that was never
declared one — so `save` refuses both, with an error saying this. A tool
that must *emit* these formats flattens under its own stated rules; the
Kubernetes agent does exactly that, and says so in its book.

## A format the crate does not read

The provider seam is still there for everything else:
`Source::provider` accepts any figment `Provider`, and
[`examples/ini_provider.rs`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/ini_provider.rs)
walks the whole plug-in — worth reading even now that INI itself is
built in, because the seam is the point of the example.

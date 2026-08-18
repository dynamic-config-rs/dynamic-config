# Migrating from 0.6

0.7 is one breaking change and two behaviours worth knowing about. Most
programs recompile without an edit.

## `Format` is `#[non_exhaustive]`

The one change that can stop a build. A `match` over
[`Format`](https://docs.rs/dynamic-config/latest/dynamic_config/enum.Format.html)
outside the crate now needs a wildcard arm:

```rust,ignore
match format {
    Format::Json => read_json(),
    Format::Toml => read_toml(),
    Format::Yaml => read_yaml(),
    other => return Err(MyError::Unsupported(other.feature())),
}
```

Why once, now: 0.7 adds `Ini` and `Properties`, and adding a variant to an
exhaustive enum is breaking whether or not the attribute is there. Marking
it `non_exhaustive` in the same release means the *next* format — and
there will be one — is additive. `Format::feature()` names the cargo
feature for any variant, so a wildcard arm can always say something
useful.

If you never match on `Format` — most callers pass it, not inspect it —
nothing changes.

## The diagnostics have a runtime seam

The stderr lines (`[dynamic-config] …: reloaded in 3ms`) are unchanged by
default, byte for byte. What is new: `set_log_sink` routes them anywhere
at runtime, `set_log_level` quiets them, and a `log` feature forwards
them to the `log` crate. The `tracing` feature still outranks everything
when compiled in.

The **language bindings** change behaviour here: the Python wheel now
delivers these lines through `logging` from the first import (see that
package's changelog), and the Node addon gains an opt-in `setLogger`.

## New formats, if you want them

```toml
dynamic-config = { version = "0.7", features = ["ini", "properties"] }
```

`config.ini` and `config.properties` then resolve, discover and watch
like any other source. Neither can be a `save` target — the error says
why. The dialect of each is specified in [Formats](../formats.md).

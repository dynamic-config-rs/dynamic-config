# dynamic-config-macros

The procedural macro behind [`dynamic-config`]. **Do not depend on this
directly** — it is an implementation detail, has no stable API of its own, and
is versioned in lockstep with the crate that re-exports it.

```toml
[dependencies]
dynamic-config = "0.2.0"    # this crate comes with it
```

Everything this crate emits is documented where a user meets it: the
[`#[dynamic_config]` attribute reference][reference] in the main README.

## Why it is a separate crate

`#[proc_macro_attribute]` requires `[lib] proc-macro = true`, and a crate with
that set can export nothing else. The split is the same one `serde` and
`serde_derive` have, for the same reason.

The generated code is deliberately thin: everything with real behaviour —
loading, merging, storage, watching — lives in `dynamic-config` as ordinary
functions that can be linted, stepped through and unit tested. Generated code
can be none of those things, so there is as little of it as possible.

[`dynamic-config`]: https://docs.rs/dynamic-config
[reference]: https://github.com/ctolon/dynamic-config#attribute-reference

## License

MIT

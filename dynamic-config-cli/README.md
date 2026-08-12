# dynamic-config-cli

Command-line diagnostics for [`dynamic-config`] configuration: the
`dynamic-config` binary.

```sh
cargo install dynamic-config-cli
```

A CLI cannot see an application's `#[dynamic_config]` attribute or its
builder, so it builds the load **from flags** — what the flags describe
must match what the application declares, or the answer is about a
different load. The flags exist because that is the honest boundary, not
a limitation to engineer away.

## `explain` — every layer's answer for one path

```sh
dynamic-config explain pool.max_size \
    --file config.toml --file secrets.toml \
    --key db --env APP_ --profile-env APP_ENV
```

Prints what every layer supplies for the path and which one wins — the
same table `AppConfig::explain(..)` renders in code. **Values print as
`***` by default**; `--show-values` opts in. A published diagnostic tool
cannot ask its user to already know which paths are sensitive, so the
safe rendering is the default and seeing values is the deliberate act.

## `diff` — which paths moved between two documents

```sh
dynamic-config diff config-yesterday.toml config-today.toml --key db
```

Paths only, never values — the audit half of a change, holding the same
line every other diagnostic in the project holds.

## Completions and the manual

```sh
dynamic-config completions bash > /etc/bash_completion.d/dynamic-config
dynamic-config man > /usr/local/share/man/man1/dynamic-config.1
```

## Stability

Experimental tier: the flag surface may still move between minor
releases. The diagnostics it prints hold the same guarantees as the
library's own.

## MSRV

1.85 — higher than [`dynamic-config`]'s own 1.71, because clap needs more
than the core does.

## License

MIT

[`dynamic-config`]: https://docs.rs/dynamic-config

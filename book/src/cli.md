# The CLI

[`dynamic-config-cli`](https://github.com/dynamic-config-rs/dynamic-config/tree/main/dynamic-config-cli)
is command-line diagnostics for configuration this crate loads: the
`dynamic-config` binary.

```sh
cargo install dynamic-config-cli
```

A CLI cannot see an application's `#[dynamic_config]` attribute or its
builder, so it builds the load **from flags** — `explain` is the
command-line form of `check()`, pointed at one path. What the flags
describe must match what the application declares, or the answer is about
a different load; the flags exist because that is the honest boundary,
not a limitation to engineer away.

## `explain` — every layer's answer for one path

```sh
dynamic-config explain pool.max_size \
    --file config.toml --file secrets.toml \
    --key db --env APP_ --profile-env APP_ENV
```

Prints what every layer supplies for the path and which one wins — the
same table `AppConfig::explain(..)` renders in code — except that
**values print as `***` by default**: a published diagnostic tool cannot
ask its user to already know which paths are sensitive, so seeing values
is the deliberate act, `--show-values`. `--env-file` adds `.env` layers,
repeatable like `--file`.

## `diff` — which paths moved between two documents

```sh
dynamic-config diff config-yesterday.toml config-today.toml --key db
```

Paths only, never values — the audit half of a change, holding the same
line every other diagnostic holds.

## Completions and the manual

```sh
dynamic-config completions bash > /etc/bash_completion.d/dynamic-config
dynamic-config man > /usr/local/share/man/man1/dynamic-config.1
```

MSRV 1.85; Beta, like every crate here — the flag surface is settled, and
the diagnostics it prints hold the library's own guarantees.

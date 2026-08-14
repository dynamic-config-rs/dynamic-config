# dynamic-config

Hot-reloadable, lock-free application configuration for Rust: one attribute
declares the type, one builder states its sources. Built on [figment].

- **crates.io:** <https://crates.io/crates/dynamic-config>
- **API documentation:** <https://docs.rs/dynamic-config>
- **Source:** <https://github.com/ctolon/dynamic-config>

## Why

Configuration in a long-running service has three awkward properties at once:
it comes from several sources with a precedence order, it is read on nearly
every request from many threads, and it should be changeable without a restart.

Doing that by hand means a `RwLock<Config>` on the read path, a bespoke file
watcher, and a reload that must not take the process down when someone saves a
broken file. This crate is all three.

|  | [`config`] | [`figment`] | Go's [Viper] | **dynamic-config** |
|---|---|---|---|---|
| Layered sources | ✅ | ✅ | ✅ | ✅ |
| Hot reload | ❌ | ❌ | ✅ | ✅ |
| Lock-free reads | — | — | **not thread-safe** | ✅ |
| Reload keeps last good config | — | — | ❌ | ✅ |
| Typed struct API | ✅ | ✅ | partial | ✅ |
| Async: await config changes | ❌ | ❌ | callback | ✅ |

The loader is figment — layered providers, profile selection and loose typing of
environment values are problems it already solves well. What this crate adds is
everything around it: the attribute and its builder, the lock-free snapshot,
the watcher, and a reload that cannot take the process down.
[Comparisons](comparisons.md) is the row-by-row version of the table above, and
[CREDITS.md](https://github.com/ctolon/dynamic-config/blob/main/CREDITS.md) is
what this engine owes to each of them.

## The shape of the crate

| | |
|---|---|
| **Three mandatory dependencies** | `figment`, `serde`, `arc-swap`. Every format, client, crypto stack and runtime is behind a feature or in a companion crate |
| **`#![forbid(unsafe_code)]`** | in every crate here, checked by CI rather than trusted |
| **MSRV 1.71** | and every feature that raises it says so, verified against real toolchains |
| **No global singleton** | each configuration type owns its storage; there is no `Config::get()` returning something a library set |
| **`no_std`** | a separate crate for microcontrollers: no filesystem, no allocator, no runtime |

## Contributing and security

[docs/CONTRIBUTOR-ONBOARDING.md](https://github.com/ctolon/dynamic-config/blob/main/docs/CONTRIBUTOR-ONBOARDING.md) is a tour of
every crate and module — what each does and where you would change it.
[CONTRIBUTING.md](https://github.com/ctolon/dynamic-config/blob/main/CONTRIBUTING.md) has what a change should carry and what is
load-bearing enough to argue about. [SECURITY.md](https://github.com/ctolon/dynamic-config/blob/main/SECURITY.md) states the
properties this crate tries to keep — and the ones it explicitly does not —
along with how to report a vulnerability privately.

`just check` runs what CI runs; `just containers` adds the suites that need a
Docker daemon.

## License

MIT

[`config`]: https://docs.rs/config
[figment]: https://docs.rs/figment
[`figment`]: https://docs.rs/figment
[Viper]: https://github.com/spf13/viper

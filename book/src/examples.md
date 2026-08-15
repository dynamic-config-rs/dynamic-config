# Examples

Twenty-eight of them, each showing one idea. All run from the workspace root.

## Getting started

| Example | Features | Shows |
|---|---|---|
| [`basic`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/basic.rs) | `json` | Load once, read the snapshot. |
| [`sections`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/sections.rs) | `json`, `watch` | Several config types over one set of files, each owning its own key, files and watcher. |
| [`errors`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/errors.rs) | `json` | Every `ErrorKind`, what each one calls for, and reading `path` and `origin`. |
| [`document_shape`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/document_shape.rs) | `json` | A file with no section header, a key the struct does not name, two files holding half a struct each, and a field nothing supplies — the four questions in [Document Shape](document-shape.md), run. |

## Where values come from

| Example | Features | Shows |
|---|---|---|
| [`layers`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/layers.rs) | `json` | One key climbing all five layers, with `source_of` naming each. |
| [`env_only`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/env_only.rs) | `json` | No files at all: the 12-factor arrangement, nested and list values included. |
| [`discovery`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/discovery.rs) | `json` | `.discover(name, paths)` across two directories, plus a profile overlay. |
| [`cli`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/cli.rs) | `clap`, `json` | Flags over the environment, `--set key=value`, and `--check` instead of booting. |

## Reloading

| Example | Features | Shows |
|---|---|---|
| [`hot_reload`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/hot_reload.rs) | `watch`, `toml` | Edit a file and watch the snapshot follow. |
| [`async_reload`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/async_reload.rs) | `async`, `watch`, `json` | A task awaiting reloads instead of polling for them. |
| [`group`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/group.rs) | `json` | Two config types reloading as one step, or not at all. |

## Getting it right

| Example | Features | Shows |
|---|---|---|
| [`validation`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/validation.rs) | `json` | Rejecting a configuration where every field is valid and the whole is not. |
| [`secrets`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/secrets.rs) | `json` | `#[config(secret)]`, and precisely what it does and does not cover. |
| [`testing`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/testing.rs) | `json` | Pinning configuration under test with the override layer. |

## With a web framework

| Example | Features | Shows |
|---|---|---|
| [`axum_hello`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/axum_hello.rs) | `watch`, `json` | A handler that reads `current()` per request, a `/config/check` probe, and why the listen port is start-up configuration. |
| [`actix_hello`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/actix_hello.rs) | `watch`, `json` | The same across Actix's worker threads, and why configuration does not belong in `web::Data`. |

## On a runtime

| Example | Features | Shows |
|---|---|---|
| [`tokio_runtime`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/tokio_runtime.rs) | `tokio`, `watch`, `json` | Two readers each waking on their own `changes()` handle, with tokio's blocking pool wired in for free. |
| [`smol_runtime`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/smol_runtime.rs) | `async`, `watch`, `json` | The whole async surface on smol, with smol's `unblock` installed as the blocking executor and no tokio in the build. |
| [`embassy_runtime`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/embassy_runtime.rs) | `async`, `json` | Embassy — an executor for microcontrollers, with no threads and no reactor — driving `changes()`, and why two rapid reloads are one wakeup. |

## Reaching further

| Example | Features | Shows |
|---|---|---|
| [`units`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/units.rs) | `json` | `"30s"` and `"64MiB"`, from files and from the environment. |
| [`generic`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/generic.rs) | `json` | `Db<Postgres>` and `Db<Mysql>` with separate snapshots. |
| [`persistence`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/persistence.rs) | `json` | Writing back atomically, and reading keys with no field. |
| [`remote`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/remote.rs) | `json` | A `RemoteSource` of your own: explicit fetch, where it sits between the layers, an unreachable store, and a watch loop pushing through its sink. |
| [`last_known_good`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/last_known_good.rs) | `json` | All three `CacheMode`s against the same broken file, with each cache file printed. |
| [`encrypted`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/encrypted.rs) | `age`, `json` | A `secrets.json.age` next to a plain `config.json`: generated key, real ciphertext, and what the wrong key looks like. |
| [`schema`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/schema.rs) | `schema`, `json` | A JSON Schema for the file two config types share, with secrets marked and `required` dropped. |
| [`no_macro`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/no_macro.rs) | `json` | `load`, `LoadSpec`, `Layer` and `ConfigCell` without the attribute. |
| [`ini_provider`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/ini_provider.rs) | `json`, `figment` | A format this crate does not read, plugged in as a source: `Source::provider` is the whole extension point, and the layer stack above it is unchanged. |

```sh
cargo run -p dynamic-config --example errors      --features json
cargo run -p dynamic-config --example document_shape --features json
cargo run -p dynamic-config --example layers      --features json
cargo run -p dynamic-config --example cli         --features clap,json -- --check
cargo run -p dynamic-config --example hot_reload  --features watch,toml
cargo run -p dynamic-config --example remote      --features json
cargo run -p dynamic-config --example last_known_good --features json
cargo run -p dynamic-config --example schema      --features schema,json
cargo run -p dynamic-config --example encrypted   --features age,json
cargo run -p dynamic-config --example axum_hello  --features watch,json
cargo run -p dynamic-config --example actix_hello --features watch,json
cargo run -p dynamic-config --example tokio_runtime --features tokio,watch,json
cargo run -p dynamic-config --example smol_runtime --features async,watch,json
cargo run -p dynamic-config --example embassy_runtime --features async,json
APP_ENV=production cargo run -p dynamic-config --example discovery --features json
```

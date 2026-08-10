# dynamic-config-embedded

Hot-reloadable configuration for `no_std` targets: no filesystem, no allocator,
no runtime.

```toml
[dependencies]
dynamic-config-embedded = { version = "0.0.1", default-features = false, features = ["json"] }
```

```rust
use dynamic_config_embedded::{ConfigCell, Format, Validate};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct Settings {
    interval_ms: u32,
    verbose: bool,
}

impl Validate for Settings {}

static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

// Compiled-in defaults, so the device is configured before anything arrives.
SETTINGS.store(Settings { interval_ms: 1000, verbose: false });

// A document from wherever this device gets one: a serial link, an MQTT
// message, a page of flash.
SETTINGS.apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)?;
```

## Why this is a separate crate

[`dynamic-config`] reads files, searches directories and merges layers with
[figment]. A microcontroller has no files, no directories and no allocator, and
figment is `std` — so this is not that crate with a feature switched off. It is
the same *shape*, built from what a device actually has.

## What it keeps

- **A snapshot in a `static`**, replaced whole. A reader never sees a
  half-applied configuration.
- **A bad document cannot take the device down.** Parsing and validation happen
  before anything is installed; a failure leaves the previous configuration
  serving.
- **`changes()`**, a `Future` that resolves on the next configuration — the same
  generation-counter-and-wakers design as the `std` crate, which is why it
  drives on Embassy, RTIC, or a hand-written executor.
- **Validation**, through the same `Validate` shape.

## What it cannot keep, and why

| | |
|---|---|
| Files, directory search, profiles | there is no filesystem |
| Environment variables | there is no environment |
| Layered merging | figment is `std`, and merging needs a value tree that allocates |
| `Arc` snapshots | no allocator; readers clone the value out |
| Provenance (`source_of`) | there is one source, so the question does not arise |

A device gets *one* document at a time and replaces the whole configuration.
That is not a reduced version of layering — it is what configuring a device
looks like.

## No allocator, and no lock a reader can block on

Storage is a `critical-section` around a plain slot: a handful of instructions
with interrupts masked, which is the primitive every embedded HAL provides and
the only one this crate needs. The section is held for a clone of the value and
nothing else.

That makes `T: Clone` the price of admission. For a configuration struct of
scalars — which is what a device's configuration is — the clone is a memcpy.

## Awaiting a change, with no allocator

`changes()` needs somewhere to keep a waker, and a `Vec<Waker>` needs an
allocator. So there are four fixed slots, chosen for the shape of the problem: a
device has a handful of tasks that care about configuration, not thousands. A
fifth waiter replaces the oldest rather than being dropped — a task that is
never woken is a hang, and one woken early merely polls again.

## Features

| Feature | Default | Effect |
|---|---|---|
| `json` | ✅ | `Format::Json`, via `serde-json-core`, which allocates nothing |
| `async` | | `changes()` |
| `std` | | the `critical-section` implementation a device gets from its HAL — for tests and host simulators |

## A field the firmware does not know is ignored

serde skips it, which on a device is the difference between a rolling upgrade
and a fleet that stops taking configuration. A firmware that wants the opposite
says so with `#[serde(deny_unknown_fields)]`.

## Testing

Unit and integration tests run on a host with the `std` feature, which supplies
the `critical-section` implementation a device gets from its HAL — the same code
otherwise. CI also builds for `thumbv7em-none-eabihf`, because "it is `no_std`"
is a claim a host build cannot check.

```sh
cargo test -p dynamic-config-embedded --features std,async
cargo check -p dynamic-config-embedded --target thumbv7em-none-eabihf \
    --no-default-features --features json,async
```

The features matter: without `std` the integration tests compile to zero
tests, silently, and the run still reports green.

## MSRV

1.83 — higher than the core crate's 1.71 because of language features, not
dependencies: `core::error::Error` in `no_std` needs 1.81 and inline `const`
blocks need 1.79. 1.83 is the floor CI verifies. Still far below anything a
device toolchain would struggle with.

## License

MIT

[`dynamic-config`]: https://docs.rs/dynamic-config
[figment]: https://docs.rs/figment

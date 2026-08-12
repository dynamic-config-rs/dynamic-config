# On a Microcontroller

[`dynamic-config-embedded`](https://github.com/ctolon/dynamic-config/tree/main/dynamic-config-embedded)
is a separate `no_std` crate: no filesystem, no allocator, no runtime.

```toml
[dependencies]
dynamic-config-embedded = { version = "<version>", default-features = false, features = ["json"] }
```

```rust
use dynamic_config_embedded::{ConfigCell, Format, Validate};

static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

// Compiled-in defaults, so the device is configured before anything arrives.
SETTINGS.store(Settings { interval_ms: 1000, verbose: false });

// A document from wherever this device gets one: a serial link, an MQTT
// message, a page of flash.
SETTINGS.apply(br#"{"interval_ms": 250, "verbose": true}"#, Format::Json)?;
```

## Why a separate crate

The core crate reads files, searches directories and merges layers with
figment. A microcontroller has none of that, and figment is `std` — so
this is **not** the core with a feature switched off. It is the same
*shape*, built from what a device actually has.

**What it keeps:** a snapshot in a `static` replaced whole (a reader
never sees a half-applied configuration); a bad document leaving the
previous configuration serving — parsing and validation run before
anything installs; validation through the same `Validate` shape; and
`changes()`, a `Future` on the same generation-counter-and-wakers design
as the `std` crate — which is why Embassy, RTIC and a hand-written
executor all drive it.

**What it cannot keep, and why:** files, discovery and profiles (no
filesystem); environment variables (no environment); layered merging
(figment is `std`, and a value tree allocates); `Arc` snapshots (no
allocator — readers clone the value out, which for a struct of scalars is
a memcpy; `T: Clone` is the price of admission); provenance (one source —
the question does not arise).

## The storage, honestly

A `critical-section` around a plain slot: a handful of instructions with
interrupts masked, the one primitive every embedded HAL provides, held
for a clone of the value and nothing else.

`changes()` needs somewhere to keep wakers and a `Vec<Waker>` needs an
allocator, so there are **four fixed slots** — a device has a handful of
tasks that care about configuration, not thousands. A fifth waiter
replaces the oldest rather than being dropped: a task never woken is a
hang; one woken early merely polls again.

## Two contracts worth knowing

- **A field the firmware does not know is ignored** — serde skips it,
  which on a device is the difference between a rolling upgrade and a
  fleet that stops taking configuration. A firmware that wants the
  opposite says `#[serde(deny_unknown_fields)]`.
- **CI builds for `thumbv7em-none-eabihf`**, because "it is `no_std`" is
  a claim a host build cannot check. Host tests run with the `std`
  feature, which supplies the `critical-section` implementation a device
  gets from its HAL.

| Feature | Default | Effect |
|---|---|---|
| `json` | ✅ | `Format::Json`, via `serde-json-core`, which allocates nothing |
| `async` | | `changes()` |
| `std` | | the `critical-section` implementation for tests and host simulators |

MSRV 1.83. The
[README](https://github.com/ctolon/dynamic-config/tree/main/dynamic-config-embedded)
carries the full story.

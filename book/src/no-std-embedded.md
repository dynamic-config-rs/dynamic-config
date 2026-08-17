# On a Microcontroller

[`dynamic-config-embedded`](https://github.com/dynamic-config-rs/dynamic-config/tree/main/dynamic-config-embedded)
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
allocator, so there are **four fixed slots** — `ConfigCell<Settings, 8>`
for eight — because a device has a handful of tasks that care about
configuration, not thousands. A fifth waiter replaces the occupant of a
slot rather than being dropped: a task never woken is a hang; one woken
early merely polls again.

## The waiter budget

Past the budget the eviction is not free. The displaced task wakes,
polls, sees no change and re-registers — displacing somebody else. Five
tasks on a four-slot cell trade wake-ups for as long as both are
waiting, with no configuration change between them, and the device never
reaches its idle loop. That is measured, not feared: the test
`one_task_past_the_budget_costs_the_device_its_idle_loop` pins it. No
wake-up is lost. The cost is power.

Three ways out were weighed, and the smallest one won.

**Raise the default.** Rejected: each slot is a `Waker`, eight bytes of
RAM on a 32-bit target, per cell, whether or not anything ever waits —
`ConfigCell<Settings, 4>` measures 56 bytes on `thumbv7em-none-eabihf`
against 88 for eight slots, for identical code size. Doubling the
default charges every device 32 bytes to move the cliff rather than
remove it: nine tasks on an eight-slot cell behave exactly as five do on
four.

**An intrusive linked list**, each future owning its node. It lifts the
cap without an allocator, and it costs `unsafe` in a crate that forbids
it, self-referential futures that must unlink under a critical section
when dropped — a use-after-free on a part with no MMU if that is ever
wrong — and *more* RAM per waiting task than a slot: a node is a `Waker`
plus its links, where a slot is the `Waker` alone.

**Take `embassy-sync`'s registration types.** They are audited and they
work, and they put a dependency in the crate whose selling point is that
it has almost none. An application that already has `embassy-sync` can
wrap a `ConfigCell` in it without this crate deciding that for everyone.

So the budget stays a compile-time number, because on a device it *is* a
compile-time number: the firmware knows its tasks. What was missing was
not capacity but a way to know the budget was wrong, since the symptom
is a battery that empties early. `waiter_evictions()` is that number —
four bytes per cell, saturating, non-zero exactly when `WAITERS` is too
small:

```rust
// On a bench, after the firmware has run everything it does.
assert_eq!(SETTINGS.waiter_evictions(), 0, "raise WAITERS");
```

## Interrupts

Every read and write of the shared state happens inside a critical
section, so storing a configuration, reading one and registering a waker
are all sound from an interrupt handler. Two rules make that true and
the tests hold them: no borrow outlives its critical section, and no
waker is ever woken while one is held — a `wake` belongs to the executor
and may store a configuration, or poll the task it just woke, before it
returns. The section itself is a scan of `WAITERS` slots and at most one
`Waker` clone; nothing on that path parses, allocates or waits.

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
[README](https://github.com/dynamic-config-rs/dynamic-config/tree/main/dynamic-config-embedded)
carries the full story.

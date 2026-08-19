# Changelog

All notable changes to `dynamic-config-embedded` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Before 1.0, a breaking change bumps the **minor** version and anything else
bumps the patch. A change to the minimum supported Rust version is breaking.

<!-- Keep this template. Add entries under `Unreleased` as you go, and move
     the whole block under a new version heading at release time.
     (Spelled `_Unreleased_` here so cargo-release's `exactly = 1` search
     for the real heading matches only the real heading.)

## [_Unreleased_]

### Added
### Changed
### Deprecated
### Removed
### Fixed
### Security

-->

## [Unreleased]

## [0.7.1] — 2026-08-19

## [0.7.0] — 2026-08-18

## [0.6.3] — 2026-08-17

## [0.6.2] — 2026-08-16

## [0.6.1] — 2026-08-14

## [0.6.0] — 2026-08-13

### Added

- `ConfigCell::waiter_evictions()` (feature `async`): a saturating count of
  registrations that had to displace another waiter. Non-zero means `WAITERS`
  is too small for this firmware — the condition that otherwise shows up only
  as a device that never idles. Four bytes of RAM per cell, 18 bytes of
  `.text` for the counter and 72 more if the accessor is called
  (`thumbv7em-none-eabihf`, `opt-level = "z"`).

### Changed

- **The waiter budget stays a compile-time number, and there will be no wait
  queue.** An intrusive list lifts the cap without an allocator and costs
  `unsafe` in a crate that forbids it, self-referential futures that must
  unlink on drop, and more RAM per waiting task than a slot — a node is a
  `Waker` plus its links, a slot is the `Waker` alone. Raising the default
  instead was measured and rejected: a slot is eight bytes on a 32-bit target,
  so `ConfigCell<Settings, 4>` is 56 bytes against 88 for eight slots at
  identical code size, and nine tasks on an eight-slot cell behave exactly as
  five do on four. A device knows its tasks at compile time, so the const
  parameter is the answer and `waiter_evictions()` says when it is set wrong.
- Documented what over-budget actually costs: not churn but a livelock. The
  displaced task wakes, re-registers and displaces another, so the executor
  never reaches its idle loop until the configuration changes. No wake-up is
  lost; the cost is power.
- Documented interrupt safety: registering a waker, storing a configuration
  and reading one are sound from an interrupt handler, because no borrow
  outlives its critical section and no waker is woken while one is held.

### Fixed

- Nothing in behaviour, but the guarantee is now tested rather than argued: a
  store that lands while a waker is registering is never a lost wake-up, every
  waiter within budget is woken exactly once per change, and both places a
  waker is woken — a configuration change and an eviction — hold no borrow
  while they do it.

## [0.5.0] — 2026-08-12

## [0.4.0] — 2026-08-12

## [0.3.0] — 2026-08-11

## [0.2.0] — 2026-08-11

### Changed

- Released in lockstep with `dynamic-config` 0.2.0, where the attribute
  declares and the builder configures. This crate's own surface is
  unchanged; its examples and docs now configure through the builder.

## [0.1.0] — 2026-08-10

### Breaking

- `ConfigCell` gains a const-generic waiter budget:
  `ConfigCell<T, const WAITERS: usize = 4>`. `WAITERS` is renamed
  `DEFAULT_WAITERS`. Un-annotated `let cell = ConfigCell::new()` now needs
  a type annotation; statics are unaffected.

### Fixed

- Trailing bytes after a JSON document are rejected — a reused link buffer
  can no longer smuggle a stale tail into an installed configuration.
- The waiter evicted by a fifth registration really is woken, and the docs
  are honest about steady-state churn beyond the budget.

## [0.0.1] — 2026-08-10

Initial release.

### Added

- `ConfigCell<T>` for `no_std` targets: a snapshot behind a
  `critical-section`, replaced whole, with parsing and validation before
  anything is installed — a bad document leaves the previous configuration
  serving.
- `apply(bytes, Format::Json)` via `serde-json-core`; no allocator anywhere.
- `changes()` as a plain `Future` over a generation counter and `WAITERS`
  fixed waker slots — Embassy, RTIC and a hand-written poll loop all drive
  it; an evicted waiter is woken, never dropped.
- The `Validate` trait, and `Error`/`ErrorKind` small enough for a status
  register.

[Unreleased]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.7.1...HEAD
[0.7.1]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.3...v0.7.0
[0.6.3]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/dynamic-config-rs/dynamic-config/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/ctolon/dynamic-config/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/ctolon/dynamic-config/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/ctolon/dynamic-config/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/ctolon/dynamic-config/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/ctolon/dynamic-config/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ctolon/dynamic-config/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ctolon/dynamic-config/compare/v0.0.1...v0.1.0
[0.0.1]: https://github.com/ctolon/dynamic-config/releases/tag/v0.0.1

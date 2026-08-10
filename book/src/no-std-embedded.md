# On a microcontroller

[`dynamic-config-embedded`](https://github.com/ctolon/dynamic-config/tree/main/dynamic-config-embedded) is a separate `no_std`
crate: no filesystem, no allocator, no runtime.

```rust
static SETTINGS: ConfigCell<Settings> = ConfigCell::new();

SETTINGS.store(Settings { interval_ms: 1000, verbose: false });   // compiled-in defaults
SETTINGS.apply(document, Format::Json)?;                          // from a link, or flash
```

It is not this crate with a feature switched off. A device has no files, no
directories and no environment, and figment is `std` — so what it keeps is the
*shape*: a snapshot in a `static` replaced whole, a bad document leaving the
previous configuration serving, validation, and `changes()` for a task that
would rather await. Storage is a `critical-section` around a plain slot, which
is the one primitive every embedded HAL provides.

CI builds it for `thumbv7em-none-eabihf`, because "it is `no_std`" is a claim a
host build cannot check.

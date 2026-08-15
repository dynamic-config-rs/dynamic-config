# Testing Your Config

Two pieces of the crate are shaped for tests; both are described in full in
their own chapters, and the
[`testing`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/testing.rs)
example shows them together.

## Pin configuration with the override layer

Overrides win over everything, which is what makes them useful in tests and
behind a `--set key=value` flag:

```rust
DbConfig::set_override("host", "localhost")?;  // a test, or --set
DbConfig::clear_overrides();
```

A test that sets an override does not care what is in the files, the
environment or a remote store — the override outranks all of them. See
[Sources & Precedence](sources-and-precedence.md#precedence) for where the
layer sits.

## Scope the watcher to the test

`builder.watch(debounce)` returns a handle, and dropping it stops the watcher.
A server calls `.detach()` to watch for the rest of the process; anything with
a lifecycle — a test, a library, a subcommand — binds the handle so watching
stops when the thing being configured goes away:

```rust
let _watch = builder.watch(debounce)?;   // a test, a subcommand: stop with the scope
```

A second `watch()` while one runs is `AlreadyExists`, so tests that share a
config type should share a watcher or scope each one. See
[Hot Reload & Watching](hot-reload.md).

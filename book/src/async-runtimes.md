# Async & Runtimes

## The `async` attribute

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, async)]
```

Generates `load_async()`, `init_async()` and `changes()`. Requires the `async`
feature — which pulls in **no runtime at all**. See [Async](#async).

## Async

The `async` feature brings in **no runtime**. `changes()` is a `Future`, so
tokio, async-std, smol and a hand-written executor all drive it identically:

```rust
#[dynamic_config(files = ["config.toml"], key = "db", watch, async)]
#[derive(Debug, Deserialize)]
struct DbConfig { pool_size: u32 }

DbConfig::init_async().await?;
DbConfig::start_watch()?.detach();

let mut changes = DbConfig::changes();

spawn(async move {
    loop {
        let config = changes.changed().await;
        pool.resize(config.pool_size);
    }
});
```

The snapshot current when `changes()` is called counts as already seen, so the
first `changed().await` waits for the *next* reload. Reloads that land while
nothing is awaiting are not queued — waking up to the latest configuration is
what a reader wants, and a queue would hand it stale ones first.

### Where the blocking work goes

Reading configuration touches the filesystem, so `load_async` moves it off the
executor. *Where* is the one genuinely runtime-specific part, so it is
pluggable:

| Setup | `load_async` uses |
|---|---|
| `tokio` feature | `tokio::task::spawn_blocking` |
| [`set_blocking_executor`] installed | that executor |
| neither | a freshly spawned thread |

A configuration load happens at startup and on reload, so a thread per call is a
real answer rather than a placeholder. For async-std or smol, hand the crate its
pool once:

```rust
struct AsyncStd;

impl BlockingExecutor for AsyncStd {
    fn execute(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        async_std::task::spawn_blocking(work);
    }
}

dynamic_config::set_blocking_executor(AsyncStd)?;
```

The watcher itself stays on a plain thread whatever you choose: `notify`'s
channel is synchronous, and keeping it off the runtime means file watching works
whether or not one is running.

[`set_blocking_executor`]: https://docs.rs/dynamic-config/latest/dynamic_config/fn.set_blocking_executor.html

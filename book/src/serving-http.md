# Serving HTTP

A web service reads configuration on nearly every request, from several
threads, while a reload may land at any moment. Three things follow from
that, and the first two need no crate beyond this one.

## Read at the use site, never at boot

```rust
async fn handler() -> String {
    let server = ServerConfig::current();   // one atomic load
    format!("{}:{}", server.host(), server.port())
}
```

Not `web::Data<ServerConfig>`, not a field on an `AppState`, not a `static`
captured at startup. Handing a snapshot to an application factory freezes it
at the moment the factory ran, and every later reload lands somewhere nobody
reads.

`current()` costs an `arc-swap` load — measured at 85 instructions and zero
allocations. Reading it per request is cheaper than the framework's own
routing.

## Some configuration is start-up configuration

```rust
let listener = TcpListener::bind((server.host(), server.port())).await?;
```

A listener cannot move to a new port without dropping every connection on
the old one. `port` reloads like everything else, and the bound socket does
not care. Say so where it is bound rather than implying otherwise: a
deployment that must change ports restarts.

The same is true of anything built once from a value — a connection pool
sized at startup, a thread count, a memory-mapped file. [The Reload
Lifecycle](reload-lifecycle.md) is about that boundary.

## Two sections in one handler

Here is where a crate earns its place. `current()` says to call it once per
request and reuse the `Arc`, and with one section that is easy:

```rust
let server = ServerConfig::current();
```

With two it is not, because *the same generation* is a property of a pair of
reads that no single call site can see:

```rust
async fn handler() -> String {
    let server = ServerConfig::current();     // generation 7
    // a reload lands here
    let features = FeaturesConfig::current(); // generation 8
    // this response now mixes two documents
}
```

Both reads are correct. The response is not.

[`dynamic-config-axum`](https://docs.rs/dynamic-config-axum) and
[`dynamic-config-actix`](https://docs.rs/dynamic-config-actix) take the
reading once, before the handler runs, and hand the result to every
extractor in it:

```toml
[dependencies]
dynamic-config-axum = "0.1"
```

```rust,ignore
use dynamic_config_axum::{Config, SnapshotLayer};
use dynamic_config_web_core::sections;

async fn handler(
    Config(server): Config<ServerConfig>,
    Config(features): Config<FeaturesConfig>,
) -> String {
    // One reading. These cannot be different generations.
    format!("{} {}", server.port(), features.cache())
}

let app = Router::new()
    .route("/", get(handler))
    .layer(SnapshotLayer::new(sections![ServerConfig, FeaturesConfig]));
```

Actix Web is the same two pieces through its own seams:

```rust,ignore
App::new()
    .wrap(DynamicConfig::new(sections![ServerConfig, FeaturesConfig]))
    .service(handler);
```

And [Loco](https://loco.rs), which is axum underneath, takes the same layer
through the `Initializer` it asks a library for — one line in `Hooks`:

```rust,ignore
async fn initializers(_ctx: &AppContext) -> Result<Vec<Box<dyn Initializer>>> {
    Ok(vec![DynamicConfig::boxed(sections![ServerConfig, FeaturesConfig])])
}
```

Loco's own `config/development.yaml` is a different thing and should stay
one: its database URL, worker mode and listen port are read once at boot,
which is right for all three. Keep the settings an operator turns during an
incident in their own file, with their own sections.

`sections![A, B]` expands to `|| A::try_current()` for each name. A
[`Dynamic<T>`](dynamic-instances.md) instance works too — register the
closure yourself with `Sections::new().section(move || handle.current())`.

**The crates own no lifecycle.** Loading, watching and the
[`WatchHandle`](hot-reload.md) stay in `main`, exactly as they were; adding
the layer changes no line of that code.

## Health and metrics

The crates ship no routes, because the pieces are already public and a
handler over them is shorter than a route surface is to adopt:

```rust,ignore
async fn readyz() -> (StatusCode, Json<Value>) {
    let status = ServerConfig::status();

    let code = if status.generation == 0 || !status.is_healthy() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };

    (code, Json(json!({ "generation": status.generation })))
}
```

Two questions, not one: `/healthz` says the process is alive and must not
fail on configuration — a process that cannot reload should stop receiving
traffic, not be restarted into reading the same broken file — while
`/readyz` is where *nothing ever loaded* and *the reloads are failing*
answer 503. [Telemetry](telemetry.md) has the whole surface, including
`Exposition` for a `/metrics` body.

## Pre-forking servers

A watcher is a thread, and a thread does not survive `fork()`. Start the
watcher **after** the fork — in a worker's own startup, not in a parent that
pre-loads the application. The parent's `init()` is inherited by the
children, which is a saving: one parse, N workers.

## The examples

| Example | Shows |
|---|---|
| [`axum_hello`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/axum_hello.rs) | one section, read per handler, with no extra crate |
| [`actix_hello`](https://github.com/dynamic-config-rs/dynamic-config/blob/main/dynamic-config/examples/actix_hello.rs) | the same across Actix's worker threads |
| [`<framework>_two_sections`](https://github.com/dynamic-config-rs/dynamic-config-web) | two sections that must agree — one per framework, and the Loco one drives the initializer through `after_routes` |

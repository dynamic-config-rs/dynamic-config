//! React to configuration changes from an async task.
//!
//! The point of `subscribe()` is that a task can *wait* for a reload rather
//! than polling `current()` on a timer — which is the difference between
//! reacting in milliseconds and reacting whenever the poll interval happens to
//! come round.
//!
//! Run from the workspace root, then edit
//! `dynamic-config/examples/config.json` in another terminal:
//!
//! ```text
//! cargo run -p dynamic-config --example async_reload --features tokio,watch,json
//! ```

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(
    files = ["dynamic-config/examples/config.json"],
    key = "server",
    env = "APP_",
    watch,
    debounce = 250,
    async
)]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `init_async` reads the files on a blocking worker, so startup does not
    // stall whatever else the runtime is already doing.
    ServerConfig::init_async().await?;
    // A server watches for as long as it runs, so nothing here owns the handle.
    ServerConfig::start_watch()?.detach();

    let initial = ServerConfig::current();
    println!("starting with {}:{}", initial.host, initial.port);

    // Take the handle before anything can change, so no reload is missed. The
    // snapshot above counts as already seen.
    let mut reloads = ServerConfig::changes();

    let listener = tokio::spawn(async move {
        loop {
            // Nothing tokio-specific: `changed()` is a `Future`, and this
            // task would read the same on async-std or smol.
            let config = reloads.changed().await;

            println!("reconfigured to {}:{}", config.host, config.port);
            // A real service would rebind, resize a pool, flip a flag…
        }
    });

    println!("edit dynamic-config/examples/config.json — ctrl-c to stop");

    listener.await?;

    Ok(())
}

//! Configuration served from somewhere other than this machine.
//!
//! ```text
//! cargo run -p dynamic-config --example remote --features json
//! ```
//!
//! The store here is a fake one, in-process, so the example needs no network.
//! A real one — [`dynamic-config-etcd`], [`dynamic-config-consul`],
//! [`dynamic-config-nats`], [`dynamic-config-vault`] — implements exactly this
//! trait and nothing more.
//!
//! [`dynamic-config-etcd`]: https://docs.rs/dynamic-config-etcd
//! [`dynamic-config-consul`]: https://docs.rs/dynamic-config-consul
//! [`dynamic-config-nats`]: https://docs.rs/dynamic-config-nats
//! [`dynamic-config-vault`]: https://docs.rs/dynamic-config-vault

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use dynamic_config::{dynamic_config, Error, Fetched, Format, RemoteSource};
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

/// The local sources the store's document layers into.
fn sources() -> dynamic_config::Builder<ServerConfig> {
    ServerConfig::builder("server")
        .file("dynamic-config/examples/app.json")
        .env("APP_")
}

/// A store that answers with whatever it was last told to, and counts reads.
///
/// The counter is the point of the example: it shows that `load()` never
/// reaches the store.
struct FakeStore {
    document: Mutex<String>,
    reads: AtomicUsize,
    reachable: Mutex<bool>,
}

impl FakeStore {
    fn new(document: &str) -> Self {
        Self {
            document: Mutex::new(document.to_owned()),
            reads: AtomicUsize::new(0),
            reachable: Mutex::new(true),
        }
    }
}

impl RemoteSource for FakeStore {
    fn fetch(&self) -> Result<Fetched, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);

        if !*self.reachable.lock().unwrap() {
            return Err(Error::remote("the store is unreachable"));
        }

        Ok(Fetched::new(
            self.document.lock().unwrap().clone(),
            Format::Json,
        ))
    }

    fn describe(&self) -> String {
        // This string is what lands in error messages, so it should name the
        // address rather than the type.
        "fake-store://in-process".to_owned()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A shared handle, only so the example can inspect the store afterwards.
    // Real code hands the source over and forgets about it.
    let store = std::sync::Arc::new(FakeStore::new(r#"{"server": {"port": 8443}}"#));

    let sources = sources();

    ServerConfig::set_remote(Handle(std::sync::Arc::clone(&store)));

    // ---------------------------------------------------------------------
    // Installing a source reaches nothing.
    // ---------------------------------------------------------------------
    println!(
        "after set_remote:   reads = {}",
        store.reads.load(Ordering::SeqCst)
    );

    let config = sources.load()?;
    println!(
        "after load:         reads = {}",
        store.reads.load(Ordering::SeqCst)
    );
    println!(
        "  host = {}, port = {} (both from the file)",
        config.host, config.port
    );

    // ---------------------------------------------------------------------
    // Fetching is the explicit step, and it is the only one that blocks.
    // ---------------------------------------------------------------------
    ServerConfig::refresh_remote()?;
    println!(
        "\nafter refresh:      reads = {}",
        store.reads.load(Ordering::SeqCst)
    );

    let config = sources.load()?;
    println!(
        "after load:         reads = {}",
        store.reads.load(Ordering::SeqCst)
    );
    println!("  port = {} (the store wins over the file)", config.port);
    // Not "an inline source": the store's own `describe` is what shows up.
    if let Some(origin) = sources.source_of("port")? {
        println!("  port comes {origin}");
    }

    // ---------------------------------------------------------------------
    // The machine's own environment still wins.
    // ---------------------------------------------------------------------
    std::env::set_var("APP_SERVER_PORT", "9999");

    let config = sources.load()?;
    println!("\nwith APP_SERVER_PORT=9999:");
    println!("  port = {} (the environment beats the store)", config.port);

    std::env::remove_var("APP_SERVER_PORT");

    // ---------------------------------------------------------------------
    // An unreachable store does not take the process down.
    // ---------------------------------------------------------------------
    *store.reachable.lock().unwrap() = false;

    let failure = ServerConfig::refresh_remote().unwrap_err();
    println!("\nstore went away:  {failure}");

    let config = sources.load()?;
    println!(
        "  port = {} — the last document it handed back is still serving",
        config.port
    );

    // Dropping it deliberately is a separate call, so it is never an accident.
    ServerConfig::clear_remote();

    let config = sources.load()?;
    println!("\nafter clear_remote:");
    println!("  port = {} (back to the file)", config.port);

    watching()?;

    Ok(())
}

/// The other half: a store that pushes, rather than one that is asked.
///
/// A real watch loop lives in the companion crate — `Etcd::watch`,
/// `Consul::watch` and the rest — and it does exactly what this loop does:
/// hand each document to `apply_remote` and let the crate do the rest.
fn watching() -> Result<(), Box<dyn std::error::Error>> {
    use dynamic_config::RemoteWatch;

    println!("\n--- watching ---\n");

    // `apply_remote` reloads through the configuration this `init` remembers,
    // so the builder step comes first.
    sources().init()?;

    ServerConfig::on_reload(|previous, current| {
        println!("  hook: port {} -> {}", previous.port, current.port);
    });

    let watch = RemoteWatch::new();
    let watching = watch.watching();

    // Stands in for a blocking query or a change stream.
    let pushes = std::thread::spawn(move || {
        for port in 9001..9020 {
            if !watching.keep_going() {
                println!("  loop: stopped after being told to");

                return;
            }

            let document =
                Fetched::new(format!(r#"{{"server": {{"port": {port}}}}}"#), Format::Json);

            // Everything a file edit would do happens here: validation, the
            // reload hooks, the cache.
            if let Err(error) = ServerConfig::apply_remote(document) {
                println!("  loop: the store pushed something unusable: {error}");
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    std::thread::sleep(std::time::Duration::from_millis(120));

    // Dropping the handle would do the same; `stop` just says it out loud.
    watch.stop();
    pushes.join().expect("the loop should end, not hang");

    println!("\nfinal port = {}", ServerConfig::current().port);

    Ok(())
}

/// `set_remote` takes ownership, so the example wraps its shared handle.
struct Handle(std::sync::Arc<FakeStore>);

impl RemoteSource for Handle {
    fn fetch(&self) -> Result<Fetched, Error> {
        self.0.fetch()
    }

    fn describe(&self) -> String {
        self.0.describe()
    }
}

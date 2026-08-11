//! The async surface on Embassy — an executor written for microcontrollers.
//!
//! ```text
//! cargo run -p dynamic-config --example embassy_runtime --features async,json
//! ```
//!
//! Embassy is about as far from tokio as an executor gets: no thread pool, no
//! reactor, no allocation in the scheduler. It is here for exactly that reason.
//! `changes()` is a hand-written `Future` over a generation counter and a list
//! of wakers — `std`, and nothing else — so if Embassy drives it, the claim that
//! this crate is runtime-agnostic means something.
//!
//! Embassy's `platform-std` runs its executor on a host, which is what makes
//! this runnable; the shape on a device is the same, minus the filesystem.
//!
//! # What an embedded program would actually do
//!
//! Not this. The builder reads files and the environment, and a
//! microcontroller has neither. What it *does* have is a remote store or a
//! serial link, and both arrive through the same door: hand a document to
//! `apply_remote`, and every reader wakes up. That path is `std`-only today —
//! see the roadmap — but the async half is already runtime-free, and this
//! example is the part that can be shown now.

use dynamic_config::{dynamic_config, Error, Fetched, Format, RemoteSource};
use embassy_executor::{Executor, Spawner};
use serde::Deserialize;
use static_cell::StaticCell;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    greeting: String,
    workers: usize,
}

/// The document the device boots with — where a real one would read flash.
///
/// It exists because `apply_remote` reloads through the configuration that
/// `init` remembered, and an `init` with nothing to load would fail: the boot
/// document is what there is to load.
struct BootDocument;

impl RemoteSource for BootDocument {
    fn fetch(&self) -> Result<Fetched, Error> {
        Ok(Fetched::new(
            r#"{"server": {"greeting": "boot", "workers": 0}}"#.to_owned(),
            Format::Json,
        ))
    }

    fn describe(&self) -> String {
        "boot-document://in-process".to_owned()
    }
}

/// A task that waits for configuration to change and does something about it.
///
/// No channel type, no runtime handle, no `Send` bound imposed by an executor:
/// it awaits a `Future` this crate returns, and Embassy polls it.
///
/// It runs forever; the pusher decides when the example is over.
#[embassy_executor::task]
async fn reader() {
    let mut changes = ServerConfig::changes();

    loop {
        let config = changes.changed().await;

        println!(
            "  reader woke: {} ({} workers)",
            config.greeting, config.workers
        );
    }
}

/// Stands in for whatever pushes configuration on a device — a remote store, a
/// serial link, a message on a bus.
#[embassy_executor::task]
async fn pusher() {
    // The reader's handle observes changes made *after* it exists, and it does
    // not exist until the reader task is first polled. Which task an executor
    // polls first is its business, not ours, so this yields before pushing
    // anything rather than assuming an order — the same reason a real program
    // starts its watch after the rest of it is up.
    embassy_futures::yield_now().await;

    println!("two pushes with no turn for the reader in between:");

    push("first", 1);
    push("second", 2);

    // A yield, not a timer: no clock, nothing a microcontroller would have to
    // configure. Two, so the reader is certainly polled before the next push.
    embassy_futures::yield_now().await;
    embassy_futures::yield_now().await;

    println!("\n  ...one wakeup, carrying the *latest* of them.");
    println!("  Reloads that land while nothing is awaiting are not queued:");
    println!("  waking to the newest configuration is what a reader wants, and");
    println!("  a queue would hand it stale ones first.\n");

    println!("and one more, with the reader waiting:");
    push("third", 3);

    embassy_futures::yield_now().await;
    embassy_futures::yield_now().await;

    println!("\nDriven by an executor with no threads, no reactor and no");
    println!("allocation in its scheduler. `changes()` is a `Future` over a");
    println!("generation counter and a list of wakers — `std`, and nothing else.");

    std::process::exit(0);
}

/// Applies one document, the way a watch loop would.
fn push(greeting: &str, workers: usize) {
    let document = format!(r#"{{"server": {{"greeting": "{greeting}", "workers": {workers}}}}}"#);

    if let Err(error) = ServerConfig::apply_remote(Fetched::new(document, Format::Json)) {
        println!("  the document did not apply: {error}");
    }
}

#[embassy_executor::task]
async fn start(spawner: Spawner) {
    // A `#[task]` hands back a token or says the pool is full; the spawner
    // itself is infallible once it has one.
    spawner.spawn(reader().expect("the task pool has room"));
    spawner.spawn(pusher().expect("the task pool has room"));
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

fn main() {
    // No files, no environment prefix: the fetched boot document is the whole
    // configuration, and initializing through the builder is what later lets
    // `apply_remote` reload.
    ServerConfig::set_remote(BootDocument);
    ServerConfig::refresh_remote().expect("the boot document is well-formed");
    ServerConfig::builder("server")
        .init()
        .expect("the boot document supplies every field");

    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner| spawner.spawn(start(spawner).expect("the task pool has room")));
}

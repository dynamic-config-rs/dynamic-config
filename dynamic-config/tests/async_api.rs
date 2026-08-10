//! The async surface: `load_async`, `init_async`, `subscribe`.
//!
//! The interesting property is not that these compile — it is that a subscriber
//! is woken by a reload it did not poll for, and that a reload landing after a
//! read does not retroactively change the value the reader already took.
//!
//! Each test gets its own config type. The snapshot lives in a `static`, so
//! sharing one type across tests would make them race each other through it.

#![cfg(all(feature = "async", feature = "json"))]

use std::time::Duration;

use dynamic_config::dynamic_config;
use serde::Deserialize;

macro_rules! server_config {
    ($name:ident) => {
        #[dynamic_config(files = ["tests/fixtures/base.json"], key = "server", async)]
        #[derive(Debug, Deserialize, PartialEq)]
        struct $name {
            #[allow(dead_code)]
            host: String,
            port: u16,
        }

        impl $name {
            // Not every generated type needs one.
            #[allow(dead_code)]
            fn replacement(port: u16) -> Self {
                Self {
                    host: "replaced".to_owned(),
                    port,
                }
            }
        }
    };
}

server_config!(LoadTarget);
server_config!(UninitTarget);
server_config!(SubscribeTarget);
server_config!(SnapshotTarget);

#[tokio::test]
async fn load_async_reads_the_same_configuration_as_load() {
    let asynchronous = LoadTarget::load_async()
        .await
        .expect("the fixture should deserialize");
    let synchronous = LoadTarget::load().expect("the fixture should deserialize");

    assert_eq!(asynchronous, synchronous);
    assert_eq!(asynchronous.port, 8080);
}

#[tokio::test]
async fn a_handle_created_before_init_waits_rather_than_failing() {
    let mut changes = UninitTarget::changes();

    // Nothing has been stored, so there is nothing to have missed.
    assert_eq!(changes.seen(), 0);

    assert!(
        tokio::time::timeout(Duration::from_millis(50), changes.changed())
            .await
            .is_err(),
        "a handle with no snapshot yet waits for the first one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscriber_is_woken_by_a_reload() {
    SubscribeTarget::init_async()
        .await
        .expect("init should succeed");

    let mut reloads = SubscribeTarget::changes();

    // The snapshot in hand counts as seen, so nothing has changed yet.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), reloads.changed())
            .await
            .is_err(),
        "changed() must not fire for the snapshot the handle started from"
    );

    let waiter = tokio::spawn(async move { reloads.changed().await.port });

    // Let the task actually park on `changed()`, so this measures a wake-up
    // rather than a value that was already sitting there.
    tokio::time::sleep(Duration::from_millis(50)).await;
    SubscribeTarget::replace(SubscribeTarget::replacement(9000));

    let observed = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the subscriber should be woken well within the timeout")
        .expect("the task should not panic");

    assert_eq!(observed, 9000);
}

#[tokio::test]
async fn a_reload_does_not_disturb_a_snapshot_already_taken() {
    SnapshotTarget::init_async()
        .await
        .expect("init should succeed");

    let held = SnapshotTarget::current();

    SnapshotTarget::replace(SnapshotTarget::replacement(9999));

    assert_eq!(
        held.port, 8080,
        "the Arc taken earlier keeps its own generation"
    );
    assert_eq!(SnapshotTarget::current().port, 9999);
}

#[test]
fn the_tokio_feature_does_not_require_a_tokio_runtime() {
    use std::future::Future;
    use std::task::{Context, Poll, Wake, Waker};

    // `#[test]`, not `#[tokio::test]`: this thread has no runtime, which is
    // the situation of a smol or async-std program that happened to compile
    // the `tokio` feature in. Dispatch must fall back to a plain thread
    // rather than panicking inside `spawn_blocking`.
    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(std::sync::Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(dynamic_config::off_thread(|| Ok(21 + 21)));

    let outcome = loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(outcome) => break outcome,
            Poll::Pending => std::thread::park(),
        }
    };

    assert_eq!(outcome.unwrap(), 42);
}

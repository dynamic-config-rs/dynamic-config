//! The async surface, driven without tokio.
//!
//! Run with `--features async,json` — no runtime crate in the graph at all.
//! The executor below is thirty lines of `std`, which is the point: `Changes`
//! and `off_thread` are a `Future` and a thread, and nothing more.

#![cfg(all(feature = "async", feature = "json", not(feature = "tokio")))]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Wake, Waker};

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
}

/// The `server` section of the base fixture.
fn server_builder() -> dynamic_config::Builder<ServerConfig> {
    ServerConfig::builder("server").file("tests/fixtures/base.json")
}

/// A whole executor: park until woken, poll again.
#[derive(Default)]
struct Parker {
    woken: Mutex<bool>,
    signal: Condvar,
}

impl Wake for Parker {
    fn wake(self: Arc<Self>) {
        *self.woken.lock().unwrap() = true;
        self.signal.notify_one();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let parker = Arc::new(Parker::default());
    let waker = Waker::from(Arc::clone(&parker));
    let mut context = Context::from_waker(&waker);

    let mut future = Box::pin(future);

    loop {
        if let Poll::Ready(output) = Pin::new(&mut future).poll(&mut context) {
            return output;
        }

        let mut woken = parker.woken.lock().unwrap();

        while !*woken {
            woken = parker.signal.wait(woken).unwrap();
        }

        *woken = false;
    }
}

#[test]
fn loading_off_thread_needs_no_runtime() {
    let config = block_on(server_builder().load_async()).expect("the fixture is complete");

    assert_eq!(config.port, 8080);
}

#[test]
fn a_change_handle_is_woken_by_a_reload() {
    block_on(server_builder().init_async()).expect("the fixture is complete");

    let mut changes = ServerConfig::changes();

    // A reload from another thread, after this one has parked.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));

        ServerConfig::replace(ServerConfig {
            host: "replaced".to_owned(),
            port: 9000,
        });
    });

    let config = block_on(changes.changed());

    assert_eq!(config.port, 9000);
}

#[test]
fn a_handle_created_before_init_sees_the_install_as_the_first_change() {
    use serde::Deserialize;

    // Its own type: the contract under test is about the moment of `init`,
    // and sharing a type another test initializes would race it away.
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct LateInit {
        #[allow(dead_code)]
        port: u16,
    }

    // Before `init`: the handle has seen nothing, so the initial install is
    // its first change — `changes()` doubles as "wake me when configuration
    // exists". Contract, not accident; see `Changes`' docs.
    let mut changes = LateInit::changes();

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));

        LateInit::builder("server")
            .file("tests/fixtures/base.json")
            .init()
            .expect("the fixture is complete");
    });

    let config = block_on(changes.changed());

    assert_eq!(config.port, 8080);
}

#[test]
fn an_error_from_the_load_still_reaches_the_waiter() {
    #[dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Missing {
        #[allow(dead_code)]
        value: u32,
    }

    let error = block_on(
        Missing::builder("nothing")
            .file("tests/fixtures/absent.json")
            .load_async(),
    )
    .expect_err("nothing supplies `value`");

    assert_eq!(error.path(), "value");
}

#[test]
fn a_panic_in_the_work_wakes_the_waiter_instead_of_hanging_it() {
    // The guard inside `off_thread` fills the slot during unwinding; without
    // it this test would park forever, which is exactly the bug it pins.
    let error = block_on(dynamic_config::off_thread::<(), _>(|| {
        panic!("the loader tripped over something")
    }))
    .unwrap_err();

    assert!(error.to_string().contains("did not finish"), "{error}");
}

#[test]
fn a_blocking_remote_source_does_not_run_on_the_executor_thread() {
    use std::sync::atomic::{AtomicU64, Ordering};

    // A blocking source that records which thread ran its fetch.
    struct Recorder(std::sync::Arc<AtomicU64>);

    impl dynamic_config::RemoteSource for Recorder {
        fn fetch(&self) -> Result<dynamic_config::Fetched, dynamic_config::Error> {
            // Thread ids have no stable integer form; hash the Debug render.
            let id = format!("{:?}", std::thread::current().id());
            self.0.store(fingerprint(&id), Ordering::SeqCst);

            Ok(dynamic_config::Fetched::new(
                r#"{"db": {}}"#,
                dynamic_config::Format::Json,
            ))
        }

        fn describe(&self) -> String {
            "a thread recorder".to_owned()
        }
    }

    fn fingerprint(text: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }

    let ran_on = std::sync::Arc::new(AtomicU64::new(0));

    let remote = dynamic_config::Remote::new();
    remote.set(Recorder(std::sync::Arc::clone(&ran_on)));

    // `block_on` drives the future on THIS thread — the stand-in for an
    // executor worker. The blocking fetch must have run somewhere else.
    block_on(remote.refresh_async()).expect("the fetch succeeds");

    let executor = fingerprint(&format!("{:?}", std::thread::current().id()));

    assert_ne!(
        ran_on.load(Ordering::SeqCst),
        executor,
        "a blocking source inside refresh_async must go through off_thread, \
         not stall the executor's worker"
    );
    assert!(remote.document().is_some());
}

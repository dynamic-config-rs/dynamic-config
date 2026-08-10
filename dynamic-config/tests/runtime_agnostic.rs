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

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "server", async)]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
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
    let config = block_on(ServerConfig::load_async()).expect("the fixture is complete");

    assert_eq!(config.port, 8080);
}

#[test]
fn a_change_handle_is_woken_by_a_reload() {
    block_on(ServerConfig::init_async()).expect("the fixture is complete");

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
fn an_error_from_the_load_still_reaches_the_waiter() {
    #[dynamic_config(files = ["tests/fixtures/absent.json"], key = "nothing", async)]
    #[derive(Debug, Deserialize)]
    struct Missing {
        #[allow(dead_code)]
        value: u32,
    }

    let error = block_on(Missing::load_async()).expect_err("nothing supplies `value`");

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

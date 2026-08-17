//! The instance engine: `Dynamic<T>` owning its storage.
//!
//! The type-level surface stores one configuration per *type*; everything
//! here pins the other contract — one per *value*: independent snapshots,
//! independent hooks, independent watchers, and the type-level machinery
//! entirely untouched by any of it.

#![cfg(feature = "json")]

use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
// The watcher's helpers and the async `changes()` loop are the only users,
// and each is behind its own feature. The workspace used to carry crates
// that turned both on for everybody, so this import looked used no matter
// what was asked for.
#[cfg(any(feature = "watch", feature = "async"))]
use std::time::Duration;

use dynamic_config::{Builder, Dynamic, Value};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Tenant {
    name: String,
    port: u16,
}

fn write_tenant(path: &str, name: &str, port: u16) {
    fs::create_dir_all("tests/scratch").unwrap();
    fs::write(
        path,
        format!(r#"{{"tenant": {{"name": "{name}", "port": {port}}}}}"#),
    )
    .unwrap();
}

#[test]
fn two_instances_of_one_type_hold_two_configurations() {
    write_tenant("tests/scratch/dyn-acme.json", "acme", 8001);
    write_tenant("tests/scratch/dyn-umbra.json", "umbra", 8002);

    let acme = Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-acme.json"));
    let umbra = Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-umbra.json"));

    // Nothing installed yet: absence is an answer, not a panic.
    assert!(acme.current().is_none());

    acme.init().unwrap();
    umbra.init().unwrap();

    assert_eq!(acme.current().unwrap().name, "acme");
    assert_eq!(umbra.current().unwrap().name, "umbra");

    // A reload of one instance is invisible to the other.
    write_tenant("tests/scratch/dyn-acme.json", "acme", 9001);
    acme.reload().unwrap();

    assert_eq!(acme.current().unwrap().port, 9001);
    assert_eq!(umbra.current().unwrap().port, 8002);
}

#[test]
fn validation_and_the_pure_load_behave_as_on_the_type_surface() {
    write_tenant("tests/scratch/dyn-validated.json", "checked", 0);

    let dynamic = Dynamic::new(
        Builder::<Tenant>::new("tenant")
            .file("tests/scratch/dyn-validated.json")
            .validate(|tenant| {
                if tenant.port == 0 {
                    return Err(dynamic_config::Error::invalid(
                        "port 0 binds to a random port",
                    ));
                }
                Ok(())
            }),
    );

    // A refused init installs nothing.
    assert!(dynamic.init().is_err());
    assert!(dynamic.current().is_none());

    // `load` is pure: fixing the file makes both work, in order.
    write_tenant("tests/scratch/dyn-validated.json", "checked", 8080);
    let loaded: Tenant = dynamic.load().unwrap();
    assert_eq!(loaded.port, 8080);
    dynamic.init().unwrap();
    assert_eq!(dynamic.current().unwrap().port, 8080);
}

#[test]
fn hooks_fire_per_instance_and_a_dropped_guard_unregisters() {
    write_tenant("tests/scratch/dyn-hooked.json", "hooked", 1);

    let dynamic =
        Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-hooked.json"));
    dynamic.init().unwrap();

    let permanent = Arc::new(AtomicU32::new(0));
    let scoped = Arc::new(AtomicU32::new(0));

    {
        let counter = Arc::clone(&permanent);
        dynamic.on_reload(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
    }
    let guard = {
        let counter = Arc::clone(&scoped);
        dynamic.on_reload_scoped(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    };

    dynamic.reload().unwrap();
    assert_eq!(permanent.load(Ordering::SeqCst), 1);
    assert_eq!(scoped.load(Ordering::SeqCst), 1);

    drop(guard);
    dynamic.reload().unwrap();
    assert_eq!(permanent.load(Ordering::SeqCst), 2);
    assert_eq!(
        scoped.load(Ordering::SeqCst),
        1,
        "the dropped guard must not fire"
    );
}

#[test]
fn the_diagnostics_answer_through_the_builder() {
    write_tenant("tests/scratch/dyn-diagnosed.json", "diagnosed", 5000);

    let dynamic =
        Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-diagnosed.json"));

    assert_eq!(dynamic.key(), "tenant");
    assert!(dynamic.builder().is_set("port").unwrap());
    assert!(matches!(
        dynamic.builder().source_of("port").unwrap(),
        Some(dynamic_config::Origin::File(_))
    ));

    let report = dynamic.builder().check().unwrap();
    assert!(report.failure.is_none());
}

#[test]
fn a_snapshot_exports_the_resolved_tree_as_owned_values() {
    write_tenant("tests/scratch/dyn-exported.json", "exported", 4242);

    let dynamic =
        Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-exported.json"));

    let value = dynamic.builder().snapshot().unwrap().to_value();

    assert_eq!(value.get("port"), Some(&Value::Integer(4242)));
    assert_eq!(
        value.get("name"),
        Some(&Value::String("exported".to_owned()))
    );
    assert_eq!(value.get("name.deeper"), None);
}

#[cfg(feature = "watch")]
mod watching {
    use super::*;
    use dynamic_config::watch::WatchMode;
    use std::time::Instant;

    /// Writes once, then waits for `read` to answer `expected`.
    ///
    /// One write is enough: the poll backend compares contents as well as
    /// timestamps, so an edit sharing a second with the scan before it is
    /// still a change. This used to rewrite in a loop, because it was not.
    fn edited_until_seen(write: impl Fn(), read: impl Fn() -> u16, expected: u16) -> bool {
        write();

        let deadline = Instant::now() + Duration::from_secs(15);

        while Instant::now() < deadline {
            if read() == expected {
                return true;
            }

            std::thread::sleep(Duration::from_millis(25));
        }

        false
    }

    #[test]
    fn two_instances_watch_side_by_side_and_double_watching_one_is_refused() {
        write_tenant("tests/scratch/dyn-watch-a.json", "a", 1);
        write_tenant("tests/scratch/dyn-watch-b.json", "b", 2);

        let a =
            Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-watch-a.json"));
        let b =
            Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-watch-b.json"));
        a.init().unwrap();
        b.init().unwrap();

        // The same *type*, two watchers — the registry keys on the instance.
        let watch_a = a
            .watch_with(
                Duration::from_millis(25),
                WatchMode::Poll {
                    interval: Duration::from_millis(50),
                },
            )
            .expect("the first instance watches");
        let _watch_b = b
            .watch_with(
                Duration::from_millis(25),
                WatchMode::Poll {
                    interval: Duration::from_millis(50),
                },
            )
            .expect("the second instance watches alongside the first");

        // One watcher per *instance*, same contract as one per type.
        let refused = a.watch(Duration::from_millis(25));
        assert_eq!(
            refused
                .expect_err("a second watch on one instance is refused")
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );

        // An edit reaches the instance that watches that file — only.
        assert!(edited_until_seen(
            || write_tenant("tests/scratch/dyn-watch-a.json", "a", 11),
            || a.current().unwrap().port,
            11,
        ));
        assert_eq!(b.current().unwrap().port, 2);

        // Dropping the handle frees the instance's slot for a fresh watch.
        drop(watch_a);
        let again = a.watch_with(
            Duration::from_millis(25),
            WatchMode::Poll {
                interval: Duration::from_millis(50),
            },
        );
        drop(again.expect("after the drop, the instance can watch again"));
    }
}

#[cfg(feature = "async")]
mod changes {
    use super::*;

    #[test]
    fn a_handle_taken_before_init_sees_the_first_install() {
        write_tenant("tests/scratch/dyn-changes.json", "woken", 7000);

        let dynamic = Arc::new(Dynamic::new(
            Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-changes.json"),
        ));

        let mut changes = dynamic.changes();
        // A hand-rolled executor on a plain thread, so this pins the
        // contract on no runtime in particular.
        let waiter = std::thread::spawn(move || {
            futures_lite_block_on(async move { changes.changed().await.port })
        });

        // Give the waiter a moment to actually be waiting, then install.
        std::thread::sleep(Duration::from_millis(100));
        dynamic.init().unwrap();

        assert_eq!(
            waiter.join().expect("the waiter thread completes"),
            7000,
            "the first install is the first change"
        );
    }

    /// A minimal executor: polls the future, parks between wakes.
    fn futures_lite_block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::sync::mpsc;
        use std::task::{Context, Poll, Wake, Waker};

        struct Unpark(mpsc::Sender<()>);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                let _ = self.0.send(());
            }
        }

        let (sender, receiver) = mpsc::channel();
        let waker = Waker::from(Arc::new(Unpark(sender)));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => {
                    receiver
                        .recv_timeout(Duration::from_secs(15))
                        .expect("the install should wake the handle");
                }
            }
        }
    }
}

/// A `Dynamic` may adopt a *generated* builder — and doing so must leave
/// the type-level surface untouched: the adopted builder installs into the
/// instance's cell, so registering it in the type's `Configured` slot
/// would cross-wire `TypeSide::reload()` into the instance while
/// `TypeSide::current()` reads a static nothing writes.
#[test]
fn adopting_a_generated_builder_does_not_capture_the_type_surface() {
    use dynamic_config::dynamic_config;

    #[dynamic_config]
    #[derive(Debug, serde::Deserialize)]
    struct TypeSide {
        port: u16,
    }

    write_tenant("tests/scratch/dyn-adopted.json", "adopted", 4100);

    let instance = Dynamic::new(TypeSide::builder("tenant").file("tests/scratch/dyn-adopted.json"));
    instance.init().expect("the instance initialises");
    assert_eq!(instance.current().expect("installed").port, 4100);

    // The type surface never saw an install…
    assert!(
        TypeSide::try_current().is_none(),
        "the instance's init must not write the type's static"
    );

    // …and it never learned a configuration either: the type-level
    // diagnostics answer through the remembered builder, and there must
    // be none to remember — not the instance's.
    assert!(
        TypeSide::source_of("port").is_err(),
        "the type's Configured slot must stay empty"
    );
    assert_eq!(instance.current().expect("still the instance's").port, 4100);
}

/// An instance's `current()` is an `Option` — there is no type name to panic
/// with — so the split pair ends in an `expect` that the joined form removes,
/// and what it returns is the install's own snapshot.
#[test]
fn init_and_current_hands_an_instance_its_snapshot_without_an_expect() {
    write_tenant("tests/scratch/dyn-paired.json", "paired", 4300);

    let instance =
        Dynamic::new(Builder::<Tenant>::new("tenant").file("tests/scratch/dyn-paired.json"));
    let tenant = instance
        .init_and_current()
        .expect("the source reads cleanly");

    assert_eq!(tenant.name, "paired");
    assert!(
        Arc::ptr_eq(&tenant, &instance.current().expect("installed above")),
        "the instance's own snapshot, not a second load of it"
    );

    // And it stays this call's snapshot when the instance moves on.
    write_tenant("tests/scratch/dyn-paired.json", "paired", 4400);
    instance.reload().expect("and reloads");

    assert_eq!(tenant.port, 4300);
    assert_eq!(instance.current().expect("installed").port, 4400);
}

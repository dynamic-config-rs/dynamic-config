//! Several configuration types reloading as one step.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, ReloadGroup};
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct DbConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
}

#[test]
fn a_group_installs_every_member() {
    // A group reloads through the builder each member was configured with,
    // so every member has to be configured before the group can move it.
    ServerConfig::builder("server")
        .file("tests/fixtures/base.json")
        .init()
        .expect("the fixture is complete");
    DbConfig::builder("db")
        .file("tests/fixtures/base.json")
        .init()
        .expect("the fixture is complete");

    let group = ReloadGroup::new().with::<ServerConfig>().with::<DbConfig>();

    assert_eq!(
        group.members().collect::<Vec<_>>(),
        ["ServerConfig", "DbConfig"]
    );

    // Plant snapshots the sources never held, so the reload proving anything
    // requires the group to have installed every member from its sources.
    ServerConfig::replace(ServerConfig {
        host: "planted".to_owned(),
        port: 1,
    });
    DbConfig::replace(DbConfig {
        host: "planted".to_owned(),
        port: 1,
    });

    group.reload().expect("the fixture is complete");

    assert_eq!(ServerConfig::current().port, 8080);
    assert_eq!(DbConfig::current().port, 5432);
}

/// The property the group exists for: one member failing leaves *every*
/// member on its previous snapshot, including the ones that loaded cleanly.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Healthy {
    port: u16,
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Fragile {
    port: u16,
}

#[test]
fn one_failure_leaves_every_member_where_it_was() {
    let group = ReloadGroup::new().with::<Healthy>().with::<Fragile>();

    // Configuring the members is what lets the group's `prepare` answer for
    // them later.
    Healthy::builder("server")
        .file("tests/fixtures/base.json")
        .init()
        .expect("both start out loadable");
    Fragile::builder("server")
        .file("tests/fixtures/base.json")
        .init()
        .expect("both start out loadable");

    group.reload().expect("both start out loadable");
    assert_eq!(Healthy::current().port, 8080);
    assert_eq!(Fragile::current().port, 8080);

    // Break only the second member, and change the first at the same time —
    // the way one edit to a shared file would.
    Healthy::set_override("port", 9999u16).unwrap();
    Fragile::set_override("port", "not-a-number").unwrap();

    let error = group.reload().expect_err("`Fragile` cannot load");
    assert!(error.path().starts_with("Fragile"), "{error}");

    assert_eq!(
        Healthy::current().port,
        8080,
        "the member that loaded cleanly must not have been installed either"
    );
    assert_eq!(Fragile::current().port, 8080);

    // And once the break is repaired, both move together.
    Fragile::clear_overrides();
    group.reload().expect("both load again");

    assert_eq!(Healthy::current().port, 9999);
    assert_eq!(Fragile::current().port, 8080);

    Healthy::clear_overrides();
}

/// A hook that panics mid-group must not stop the remaining members'
/// snapshots from installing — a half-committed group is the exact state the
/// type promises against.
#[test]
fn a_panicking_commit_does_not_interrupt_the_other_commits() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COMMITTED: AtomicUsize = AtomicUsize::new(0);

    struct Panicking;
    struct Quiet;

    impl dynamic_config::Reloadable for Panicking {
        fn prepare() -> Result<dynamic_config::Commit, dynamic_config::Error> {
            Ok(Box::new(|| {
                COMMITTED.fetch_add(1, Ordering::SeqCst);
                panic!("a bug in a reload hook, surfacing during commit");
            }))
        }

        fn name() -> &'static str {
            "Panicking"
        }
    }

    impl dynamic_config::Reloadable for Quiet {
        fn prepare() -> Result<dynamic_config::Commit, dynamic_config::Error> {
            Ok(Box::new(|| {
                COMMITTED.fetch_add(1, Ordering::SeqCst);
            }))
        }

        fn name() -> &'static str {
            "Quiet"
        }
    }

    let group = dynamic_config::ReloadGroup::new()
        .with::<Panicking>()
        .with::<Quiet>();

    group
        .reload()
        .expect("prepare succeeded for every member, so the reload reports Ok");

    assert_eq!(
        COMMITTED.load(Ordering::SeqCst),
        2,
        "the member after the panicking one must still commit"
    );
}

/// Two threads reloading the same group must serialize: their commit loops
/// interleaving is how member A ends up on thread 1's prepare and member B
/// on thread 2's.
#[test]
fn concurrent_reloads_serialize() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    static IN_COMMIT: AtomicUsize = AtomicUsize::new(0);
    static OVERLAPS: AtomicUsize = AtomicUsize::new(0);

    struct Slow;

    impl dynamic_config::Reloadable for Slow {
        fn prepare() -> Result<dynamic_config::Commit, dynamic_config::Error> {
            Ok(Box::new(|| {
                // If another thread's commit loop is running right now, the
                // serialization failed.
                if IN_COMMIT.fetch_add(1, Ordering::SeqCst) > 0 {
                    OVERLAPS.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
                IN_COMMIT.fetch_sub(1, Ordering::SeqCst);
            }))
        }

        fn name() -> &'static str {
            "Slow"
        }
    }

    let group = Arc::new(dynamic_config::ReloadGroup::new().with::<Slow>());

    let barrier = Arc::new(Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let group = Arc::clone(&group);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                group.reload().unwrap();
            })
        })
        .collect();

    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(
        OVERLAPS.load(Ordering::SeqCst),
        0,
        "commit loops from different threads must never overlap"
    );
}

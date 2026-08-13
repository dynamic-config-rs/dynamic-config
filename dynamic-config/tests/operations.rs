//! Why a snapshot was installed, and how the reloads since have gone.
//!
//! Each [`ReloadReason`] is asserted against the path that is supposed to
//! produce it — a reason nothing produces is worse than no reason at all,
//! because it reads as an answer. The status half is here too: what a
//! failed reload does to the counters, and what a successful one undoes.
//!
//! One configuration type and one fixture path per test: these run in
//! parallel, and a type's snapshot lives in a `static` keyed by the type.

#![cfg(feature = "json")]

use std::sync::{Arc, Mutex};

use dynamic_config::{dynamic_config, Builder, Dynamic, ReloadReason};
use serde::Deserialize;

fn write(path: &str, port: u16) {
    std::fs::create_dir_all("tests/scratch").expect("the scratch directory is creatable");
    std::fs::write(path, format!(r#"{{"svc": {{"port": {port}}}}}"#))
        .expect("the scratch file is writable");
}

fn write_broken(path: &str) {
    std::fs::write(path, r#"{"svc": {"port": "not-a-number"}}"#)
        .expect("the scratch file is writable");
}

// ---------------------------------------------------------------------------
// One reason per producing path
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Svc {
    #[allow(dead_code)]
    port: u16,
}

/// `init()` is `Initial` and a later `reload()` is `Manual`: the two calls
/// a program makes by hand, told apart.
#[test]
fn init_is_initial_and_a_hand_reload_is_manual() {
    let path = "tests/scratch/ops-initial.json";
    write(path, 8101);

    let svc = Dynamic::new(Builder::<Svc>::new("svc").file(path));
    let seen = Arc::new(Mutex::new(Vec::new()));

    {
        let recorder = Arc::clone(&seen);
        svc.on_reload_with(move |event| {
            recorder
                .lock()
                .unwrap()
                .push((event.reason.clone(), event.previous.is_some()));
        });
    }

    svc.init().expect("the fixture loads");
    svc.reload().expect("and reloads");

    assert_eq!(
        *seen.lock().unwrap(),
        [(ReloadReason::Initial, false), (ReloadReason::Manual, true),],
        "the first install has no previous snapshot, and says so"
    );
    assert_eq!(svc.status().last_reason, Some(ReloadReason::Manual));
}

#[derive(Debug, Deserialize)]
struct Recovering {
    port: u16,
}

/// Starting from the last-known-good cache is its own reason: an operator
/// reading `Manual` here would think somebody asked for this configuration.
#[test]
fn a_start_from_the_cache_is_recovered() {
    let path = "tests/scratch/ops-recovered.json";
    let cache = "tests/scratch/ops-recovered-cache.json";
    let _ = std::fs::remove_file(cache);
    write(path, 8102);

    let good = Dynamic::new(
        Builder::<Recovering>::new("svc")
            .file(path)
            .cache(cache, dynamic_config::CacheMode::Full),
    );
    good.init().expect("a clean start writes the cache");
    assert_eq!(good.status().last_reason, Some(ReloadReason::Initial));

    // A second instance over a broken source, with the same cache to fall
    // back on: the load fails, the cache stands in, and the reason says so.
    write_broken(path);

    let fallen_back = Dynamic::new(
        Builder::<Recovering>::new("svc")
            .file(path)
            .cache(cache, dynamic_config::CacheMode::Full),
    );
    fallen_back
        .init()
        .expect("the cache carries the last good configuration");

    assert_eq!(
        fallen_back.status().last_reason,
        Some(ReloadReason::Recovered)
    );
    assert_eq!(fallen_back.current().unwrap().port, 8102);
    assert_eq!(
        fallen_back.status().consecutive_failures,
        0,
        "a recovery installed something, so it is not a failure"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Pushed {
    #[allow(dead_code)]
    port: u16,
}

/// A document a store pushed through `RemoteSink::apply` reloads through
/// the same builder a file edit would — and has to be distinguishable from
/// one, which is the whole point of the reason.
#[test]
fn a_remote_push_is_remote_changed() {
    let path = "tests/scratch/ops-remote.json";
    write(path, 8103);

    Pushed::builder("svc")
        .file(path)
        .init()
        .expect("the fixture loads");

    let seen = Arc::new(Mutex::new(Vec::new()));
    {
        let recorder = Arc::clone(&seen);
        Pushed::on_reload_with(move |event| recorder.lock().unwrap().push(event.reason.clone()));
    }

    Pushed::remote_sink()
        .apply(dynamic_config::Fetched::new(
            r#"{"svc": {"port": 9103}}"#,
            dynamic_config::Format::Json,
        ))
        .expect("the pushed document loads");

    assert_eq!(*seen.lock().unwrap(), [ReloadReason::RemoteChanged]);
    assert_eq!(
        Pushed::status().last_reason,
        Some(ReloadReason::RemoteChanged)
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Labelled {
    #[allow(dead_code)]
    port: u16,
}

/// A program that detects its own changes has somewhere to say so, rather
/// than every reload it drives arriving as `Manual`.
#[test]
fn reload_with_carries_the_callers_own_label() {
    let path = "tests/scratch/ops-labelled.json";
    write(path, 8104);

    let builder = Labelled::builder("svc").file(path);
    builder.init().expect("the fixture loads");

    builder
        .reload_with(ReloadReason::RemoteChanged)
        .expect("and reloads");

    assert_eq!(
        Labelled::status().last_reason,
        Some(ReloadReason::RemoteChanged)
    );
}

// ---------------------------------------------------------------------------
// status()
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Counted {
    #[allow(dead_code)]
    port: u16,
}

/// The counter is the health and the record is the history: failures
/// accumulate, an install clears the streak, and what went wrong last stays
/// readable afterwards.
#[test]
fn consecutive_failures_count_up_and_reset_on_an_install() {
    let path = "tests/scratch/ops-counted.json";
    write(path, 8105);

    let builder = Counted::builder("svc").file(path);
    builder.init().expect("the fixture loads");

    let status = Counted::status();
    assert_eq!(status.generation, 1);
    assert!(status.is_healthy());
    assert!(status.last_failure.is_none());
    assert!(status.stale_for().is_some());

    write_broken(path);

    for expected in 1..=2 {
        builder.reload().expect_err("the port is not a number");
        assert_eq!(Counted::status().consecutive_failures, expected);
    }

    let failed = Counted::status();
    assert!(!failed.is_healthy());
    assert_eq!(
        failed.generation, 1,
        "a failed reload installs nothing, so the generation stands still"
    );
    let failure = failed.last_failure.expect("two reloads failed");
    assert_eq!(failure.kind, dynamic_config::ErrorKind::Type);
    assert_eq!(failure.path, "port", "the key is the actionable half");

    write(path, 9105);
    builder.reload().expect("the fixture is well formed again");

    let healed = Counted::status();
    assert_eq!(healed.consecutive_failures, 0);
    assert_eq!(healed.generation, 2);
    assert!(
        healed.last_failure.is_some(),
        "the streak resets; the record of what went wrong does not"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Refused {
    #[allow(dead_code)]
    port: u16,
}

/// A start that cannot load at all still leaves a status worth reading:
/// nothing installed, and a failure saying why.
#[test]
fn a_start_that_never_installed_reports_a_failure_and_no_generation() {
    let path = "tests/scratch/ops-refused.json";
    write_broken(path);

    Refused::builder("svc")
        .file(path)
        .init()
        .expect_err("the port is not a number and there is no cache");

    let status = Refused::status();

    assert_eq!(status.generation, 0);
    assert!(status.loaded_at.is_none());
    assert!(status.last_reason.is_none());
    assert!(status.stale_for().is_none());
    assert_eq!(status.consecutive_failures, 1);
    assert_eq!(
        status.last_failure.expect("the load failed").kind,
        dynamic_config::ErrorKind::Type
    );
}

// ---------------------------------------------------------------------------
// The two hook forms over one list
// ---------------------------------------------------------------------------

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Both {
    #[allow(dead_code)]
    port: u16,
}

/// Both forms fire, once each per reload, in the order they were
/// registered — one list, one dispatch loop.
#[test]
fn both_hook_forms_fire_once_each_in_registration_order() {
    let path = "tests/scratch/ops-both.json";
    write(path, 8106);

    let order = Arc::new(Mutex::new(Vec::new()));

    {
        let recorder = Arc::clone(&order);
        Both::on_reload_with(move |_| recorder.lock().unwrap().push("event-first"));
    }
    {
        let recorder = Arc::clone(&order);
        Both::on_reload(move |_, _| recorder.lock().unwrap().push("pair"));
    }
    {
        let recorder = Arc::clone(&order);
        Both::on_reload_with(move |_| recorder.lock().unwrap().push("event-last"));
    }

    let builder = Both::builder("svc").file(path);
    builder.init().expect("the fixture loads");

    assert_eq!(
        *order.lock().unwrap(),
        ["event-first", "event-last"],
        "the pair form has nowhere to put `there was none`, so the first \
         install does not reach it"
    );

    order.lock().unwrap().clear();
    builder.reload().expect("and reloads");

    assert_eq!(
        *order.lock().unwrap(),
        ["event-first", "pair", "event-last"]
    );
}

// ---------------------------------------------------------------------------
// The watcher's reason carries the file
// ---------------------------------------------------------------------------

#[cfg(feature = "watch")]
mod watching {
    use super::{write, Arc, Deserialize, Mutex, ReloadReason};
    use dynamic_config::dynamic_config;
    use std::time::{Duration, Instant};

    const PATH: &str = "tests/scratch/ops-watched.json";

    #[dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Watched {
        port: u16,
    }

    /// The path the watcher saw is dropped nowhere: it reaches the hook as
    /// `FileChanged`, which is what lets a program watching two files log
    /// which one moved.
    #[test]
    fn a_watched_edit_names_the_file_that_changed() {
        write(PATH, 8107);

        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let recorder = Arc::clone(&seen);
            Watched::on_reload_with(move |event| {
                recorder.lock().unwrap().push(event.reason.clone());
            });
        }

        let builder = Watched::builder("svc").file(PATH);
        builder.init().expect("the fixture loads");

        let _watch = builder
            .watch(Duration::from_millis(50))
            .expect("the watcher thread spawns");

        write(PATH, 9107);

        // Generous, and polled rather than slept: a wedged watcher should
        // fail the test rather than hang CI.
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && Watched::current().port != 9107 {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(Watched::current().port, 9107, "the edit should reload");

        let seen = seen.lock().unwrap();
        let reason = seen
            .iter()
            .find(|reason| matches!(reason, ReloadReason::FileChanged(_)))
            .expect("the watcher's reload is a file change");

        let ReloadReason::FileChanged(path) = reason else {
            unreachable!("just matched")
        };
        assert!(
            path.ends_with("ops-watched.json"),
            "the reason must name the file that changed, got {}",
            path.display()
        );
        assert!(matches!(
            Watched::status().last_reason,
            Some(ReloadReason::FileChanged(_))
        ));
    }
}

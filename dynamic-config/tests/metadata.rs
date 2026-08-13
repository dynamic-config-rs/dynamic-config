//! What is true of the installed snapshot, for the operator asking.
//!
//! Two questions with no API before this: *which generation is live* and
//! *how stale is it*. The rule the whole design hangs on is pinned here
//! too — metadata is a load of its own, so reading configuration stays one
//! atomic load with nothing to project out of it.

#![cfg(feature = "json")]

use std::fs;
use std::thread;
use std::time::Duration;

use dynamic_config::{Builder, ConfigCell, Dynamic};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Tenant {
    #[allow(dead_code)]
    name: String,
    port: u16,
}

fn write_tenant(path: &str, port: &str) {
    fs::create_dir_all("tests/scratch").unwrap();
    fs::write(
        path,
        format!(r#"{{"tenant": {{"name": "acme", "port": {port}}}}}"#),
    )
    .unwrap();
}

#[test]
fn generation_counts_installs_and_starts_at_zero() {
    let path = "tests/scratch/meta-generation.json";
    write_tenant(path, "8001");

    let tenant = Dynamic::new(Builder::<Tenant>::new("tenant").file(path));

    assert_eq!(
        tenant.generation(),
        0,
        "zero is never a generation, so it can mean `nothing installed`"
    );
    assert!(tenant.meta().is_none());

    tenant.init().unwrap();
    assert_eq!(tenant.generation(), 1);

    for expected in 2..=4 {
        tenant.reload().unwrap();
        assert_eq!(tenant.generation(), expected);
    }

    assert_eq!(tenant.meta().unwrap().generation, 4);
}

#[test]
fn loaded_at_moves_on_an_install_and_stays_put_on_a_refusal() {
    let path = "tests/scratch/meta-loaded-at.json";
    write_tenant(path, "8002");

    let tenant = Dynamic::new(Builder::<Tenant>::new("tenant").file(path));
    tenant.init().unwrap();

    let first = tenant.meta().expect("init installed a snapshot");

    // The clock is monotonic, so a real pause is a real difference rather
    // than a hope about resolution.
    thread::sleep(Duration::from_millis(5));
    tenant.reload().unwrap();

    let second = tenant.meta().expect("the reload installed another");
    assert!(second.loaded_at > first.loaded_at);
    assert_eq!(second.generation, first.generation + 1);

    // A reload that fails installs nothing, so it moves neither number —
    // "how stale is this" has to answer for the snapshot actually serving.
    write_tenant(path, r#""not-a-number""#);
    thread::sleep(Duration::from_millis(5));
    tenant.reload().unwrap_err();

    assert_eq!(
        tenant.meta(),
        Some(second),
        "a refused reload leaves the previous snapshot, and its metadata, alone"
    );
    assert_eq!(tenant.current().unwrap().port, 8002);
}

/// The counter is allocated by the same compare-and-swap that publishes it,
/// so overlapping reloads cannot lose one or hand out a number twice.
#[test]
fn every_install_is_counted_when_stores_overlap() {
    const THREADS: u64 = 4;
    const PER_THREAD: u64 = 250;

    let cell: &'static ConfigCell<u64> = Box::leak(Box::new(ConfigCell::new()));

    let writers: Vec<_> = (0..THREADS)
        .map(|writer| {
            thread::spawn(move || {
                for step in 0..PER_THREAD {
                    cell.store(writer * PER_THREAD + step);
                }
            })
        })
        .collect();

    for writer in writers {
        writer.join().unwrap();
    }

    assert_eq!(cell.generation(), THREADS * PER_THREAD);
}

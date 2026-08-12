//! The engine's numbers, measured the way a skeptic would ask for them.
//!
//! `read_path.rs` stays as the hand-rolled loop the README quotes — honest
//! about what it is. This file is the statistics-driven upgrade the roadmap
//! promised: criterion, so a stranger gets confidence intervals rather than
//! one run's average, over the four questions that decide whether this
//! crate is fit for a hot path:
//!
//! - **The read.** What `current()` costs on each storage shape — the
//!   `static` cell, the `TypeId` registry, a `Dynamic` instance.
//! - **The read under fire.** The same read while another thread installs
//!   new snapshots as fast as it can — the concurrent
//!   readers-during-reload scenario, which is the number a server actually
//!   lives on.
//! - **The reload.** `Dynamic::reload()` end to end — read the file, parse,
//!   deserialize, validate, swap — which is the latency an edit takes to
//!   become visible, minus only the watcher's debounce.
//! - **The scale.** A pure `load()` against generated documents of one
//!   hundred, ten thousand and one hundred thousand keys, because "fast on
//!   a ten-line file" is not a claim about configuration at fleet size.
//!
//! ```text
//! cargo bench -p dynamic-config --features json --bench engine
//! ```
//!
//! Cross-library comparisons stay out of here deliberately: they rot too
//! fast to gate on, and belong in a written-up experiment.

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use dynamic_config::{dynamic_config, Builder, Dynamic};
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Plain {
    port: u16,
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Generic<T> {
    port: u16,
    #[serde(skip)]
    marker: std::marker::PhantomData<fn() -> T>,
}

struct Marker;

#[derive(Debug, Clone, Deserialize)]
struct Instance {
    port: u16,
}

/// One flat section of `keys` string values, as a JSON document on disk.
///
/// Flat rather than nested: the question is how the loader scales with
/// *entries*, and nesting only decides which map they land in.
fn document_with(keys: usize) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("dynamic-config-bench-{keys}.json"));

    let mut body = String::with_capacity(keys * 32);
    body.push_str(r#"{"wide": {"#);
    for key in 0..keys {
        if key > 0 {
            body.push(',');
        }
        let _ = write!(body, r#""key_{key}": "value {key}""#);
    }
    body.push_str("}}");

    std::fs::write(&path, body).expect("the bench document writes");
    path
}

fn reads(criterion: &mut Criterion) {
    Plain::builder("db")
        .file("benches/bench.json")
        .init()
        .expect("benches/bench.json should load");
    Generic::<Marker>::builder("db")
        .file("benches/bench.json")
        .init()
        .expect("benches/bench.json should load");
    let dynamic = Dynamic::new(Builder::<Instance>::new("db").file("benches/bench.json"));
    dynamic.init().expect("benches/bench.json should load");

    let mut group = criterion.benchmark_group("read");
    group.bench_function("static", |bencher| {
        bencher.iter(|| black_box(Plain::current().port));
    });
    group.bench_function("registry", |bencher| {
        bencher.iter(|| black_box(Generic::<Marker>::current().port));
    });
    group.bench_function("dynamic", |bencher| {
        bencher.iter(|| black_box(dynamic.current().expect("initialised").port));
    });
    group.finish();

    // The same read while a writer installs snapshots as fast as it can:
    // the reload path is `ArcSwap::store`, so what this pins is that a
    // reload never makes readers wait — the property the crate is for.
    let mut group = criterion.benchmark_group("read_during_reload");
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                Plain::replace(Plain { port: 8080 });
            }
        })
    };
    group.bench_function("static", |bencher| {
        bencher.iter(|| black_box(Plain::current().port));
    });
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.join().expect("the writer thread completes");
    group.finish();
}

fn reloads(criterion: &mut Criterion) {
    let path = document_with(10);
    let dynamic =
        Dynamic::new(Builder::<serde_json::Value>::new("wide").file(path.display().to_string()));
    dynamic.init().expect("the generated document loads");

    // File to visible snapshot, everything included except the watcher's
    // debounce: read, parse, deserialize, validate, swap.
    criterion.bench_function("reload", |bencher| {
        bencher.iter(|| dynamic.reload().expect("the reload succeeds"));
    });
}

fn scale(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("load_scale");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(8));

    for keys in [100usize, 10_000, 100_000] {
        let path = document_with(keys);
        let builder = Builder::<serde_json::Value>::new("wide").file(path.display().to_string());

        group.bench_with_input(BenchmarkId::from_parameter(keys), &keys, |bencher, _| {
            bencher.iter(|| black_box(builder.load().expect("the document loads")));
        });
    }

    group.finish();
}

criterion_group!(benches, reads, reloads, scale);
criterion_main!(benches);

//! The dimensions `engine.rs` does not sweep: merge depth, reader fan-out
//! and watcher count. Together the two files are the benchmark matrix the
//! perf-budget page quotes; CI compiles both (`--no-run`) and a human runs
//! them where the numbers matter.
//!
//! ```text
//! cargo bench -p dynamic-config --features json,watch --bench matrix
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dynamic_config::{load, ConfigCell, Format, LoadSpec, Source};

/// One JSON layer with `keys` keys, each scoped so overlays overlap.
fn layer_text(keys: usize, salt: usize) -> String {
    let mut body = String::from("{");

    for k in 0..keys {
        if k > 0 {
            body.push(',');
        }

        body.push_str(&format!("\"k{k}\": {}", k + salt));
    }

    body.push('}');
    body
}

/// merge × layer-count: resolving 2/4/8/16 fully-overlapping layers of a
/// 64-key document. The cost that grows here is the merge itself.
fn merge_layers(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("merge_layers");

    for layers in [2usize, 4, 8, 16] {
        let texts: Vec<String> = (0..layers).map(|salt| layer_text(64, salt)).collect();

        group.throughput(Throughput::Elements(layers as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(layers),
            &texts,
            |bench, texts| {
                bench.iter(|| {
                    let sources: Vec<Source> = texts
                        .iter()
                        .map(|text| Source::inline(text, Format::Json))
                        .collect();
                    let spec = LoadSpec::new("doc", &sources).with_whole_document(true);

                    load::<serde_json::Value>(&spec).expect("the layers resolve")
                });
            },
        );
    }

    group.finish();
}

/// readers × N: aggregate reads per second while 1/8/32/128 threads hammer
/// one cell and a writer installs continuously. Not a criterion timing loop
/// — thread startup would swamp it — but a fixed-window throughput count
/// reported through criterion's machinery for the record.
fn readers_scaling(criterion: &mut Criterion) {
    static CELL: ConfigCell<u64> = ConfigCell::new();

    CELL.store(0);

    let mut group = criterion.benchmark_group("readers_scaling");
    group.sample_size(10);

    for readers in [1usize, 8, 32, 128] {
        group.bench_with_input(
            BenchmarkId::from_parameter(readers),
            &readers,
            |bench, &readers| {
                bench.iter_custom(|iterations| {
                    let stop = Arc::new(AtomicBool::new(false));

                    let writer = {
                        let stop = Arc::clone(&stop);
                        std::thread::spawn(move || {
                            let mut n = 0u64;

                            while !stop.load(Ordering::Relaxed) {
                                CELL.store(n);
                                n += 1;
                            }
                        })
                    };

                    let workers: Vec<_> = (0..readers)
                        .map(|_| {
                            let stop = Arc::clone(&stop);
                            std::thread::spawn(move || {
                                let mut reads = 0u64;

                                while !stop.load(Ordering::Relaxed) {
                                    criterion::black_box(CELL.load());
                                    reads += 1;
                                }

                                reads
                            })
                        })
                        .collect();

                    // One fixed window per criterion iteration batch.
                    let window = Duration::from_millis(50 * iterations.clamp(1, 20));
                    let started = Instant::now();
                    std::thread::sleep(window);
                    stop.store(true, Ordering::Relaxed);

                    let _total: u64 = workers.into_iter().map(|w| w.join().unwrap()).sum();
                    writer.join().unwrap();

                    started.elapsed()
                });
            },
        );
    }

    group.finish();
}

criterion_group!(matrix, merge_layers, readers_scaling);
criterion_main!(matrix);

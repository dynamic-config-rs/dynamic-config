//! What `current()` costs, on each of the three storage shapes — and what
//! reading by *path* costs instead of by field.
//!
//! The question this answers is whether generic configuration types can share
//! the non-generic read path or need one of their own. A non-generic type keeps
//! its snapshot in a `static` — one atomic load. A generic one cannot, because
//! Rust has no generic statics, so it goes through a `TypeId`-keyed registry:
//! a read lock, a hash, and a downcast before the same atomic load. A
//! `Dynamic` instance owns its cell behind an `Arc` — the same atomic load,
//! one pointer hop earlier.
//!
//! The second group is the schemaless shape, and it exists because
//! "`config.get("db.pool.max_size")`" cannot be the same number as reading a
//! field and the crate should not let anyone assume it is. A path read is
//! the same atomic load followed by a walk: split the path, one `BTreeMap`
//! lookup per segment. A *typed* path read is that plus a rebuild of the
//! value and a serde run, on every call. Both are measured against the field
//! read they are being compared with, on the same machine in the same run,
//! which is the only comparison worth publishing.
//!
//! ```text
//! cargo bench -p dynamic-config --features json
//! ```
//!
//! Deliberately not `criterion`: the gap being measured is the difference
//! between one atomic load and a hashed lookup, which is large enough that a
//! loop and a clock resolve it, and a heavyweight dependency to confirm a ratio
//! this size would cost more than it settles.

use std::hint::black_box;
use std::time::Instant;

use dynamic_config::dynamic_config;
use serde::Deserialize;

/// Rounds, chosen so a run takes a second or two rather than a moment.
const ROUNDS: u32 = 5_000_000;

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

fn time(label: &str, mut work: impl FnMut()) -> f64 {
    // Warm up: the registry's first lookup allocates, and neither shape should
    // be judged on its first call.
    for _ in 0..1_000 {
        work();
    }

    let started = Instant::now();

    for _ in 0..ROUNDS {
        work();
    }

    let per_call = started.elapsed().as_secs_f64() / f64::from(ROUNDS) * 1e9;

    println!("{label:<28} {per_call:>8.2} ns/read");

    per_call
}

#[derive(Debug, Deserialize)]
struct Instance {
    port: u16,
}

/// Where these numbers came from, printed with them.
///
/// A benchmark result travels — into a changelog, into a book page, into
/// somebody's decision. Without the machine it ran on it is a number
/// pretending to be a fact, and the ratios are what survive the trip anyway.
/// The same block the Python benchmark prints, with what a Rust program can
/// actually see at runtime.
fn environment() -> String {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, model)| model.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let cores =
        std::thread::available_parallelism().map_or("unknown".to_owned(), |n| n.to_string());

    format!(
        "  cpu         {cpu}\n  cores       {cores}\n  target      {}-{}\n  \
         build       release\n  rounds      {ROUNDS} per measurement",
        std::env::consts::ARCH,
        std::env::consts::OS,
    )
}

fn main() {
    println!("measured on\n\n{}\n", environment());

    Plain::builder("db")
        .file("benches/bench.json")
        .init()
        .expect("benches/bench.json should load");
    Generic::<Marker>::builder("db")
        .file("benches/bench.json")
        .init()
        .expect("benches/bench.json should load");

    let plain = time("static ConfigCell", || {
        black_box(Plain::current().port);
    });

    let generic = time("registry (generic)", || {
        black_box(Generic::<Marker>::current().port);
    });

    let dynamic_config = dynamic_config::Dynamic::new(
        dynamic_config::Builder::<Instance>::new("db").file("benches/bench.json"),
    );
    dynamic_config
        .init()
        .expect("benches/bench.json should load");

    let dynamic = time("Dynamic (instance)", || {
        black_box(dynamic_config.current().expect("initialised above").port);
    });

    println!("\ngeneric / static: {:.1}x", generic / plain);
    println!("dynamic / static: {:.1}x", dynamic / plain);

    // ---------------------------------------------------------------------
    // The schemaless shape: the same storage, read by path instead of field.
    // ---------------------------------------------------------------------
    let schemaless = dynamic_config::Dynamic::new(
        dynamic_config::Builder::values("db").file("benches/bench.json"),
    );
    schemaless.init().expect("benches/bench.json should load");

    println!();

    let shallow = time("Value::get, one segment", || {
        let values = schemaless.current().expect("initialised above");

        black_box(values.get("port").and_then(dynamic_config::Value::as_i64));
    });

    let nested = time("Value::get, two segments", || {
        let values = schemaless.current().expect("initialised above");

        black_box(
            values
                .get("pool.max_size")
                .and_then(dynamic_config::Value::as_i64),
        );
    });

    let typed = time("Value::get_as::<u16>", || {
        let values = schemaless.current().expect("initialised above");

        black_box(values.get_as::<u16>("pool.max_size").expect("it is there"));
    });

    println!("\npath (1 segment) / static field: {:.1}x", shallow / plain);
    println!("path (2 segments) / static field: {:.1}x", nested / plain);
    println!("get_as / static field:            {:.1}x", typed / plain);
}

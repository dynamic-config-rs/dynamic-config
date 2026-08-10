//! What `current()` costs, on each of the two storage shapes.
//!
//! The question this answers is whether generic configuration types can share
//! the non-generic read path or need one of their own. A non-generic type keeps
//! its snapshot in a `static` — one atomic load. A generic one cannot, because
//! Rust has no generic statics, so it goes through a `TypeId`-keyed registry:
//! a read lock, a hash, and a downcast before the same atomic load.
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

#[dynamic_config(files = ["benches/bench.json"], key = "db")]
#[derive(Debug, Deserialize)]
struct Plain {
    port: u16,
}

#[dynamic_config(files = ["benches/bench.json"], key = "db")]
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

fn main() {
    Plain::init().expect("benches/bench.json should load");
    Generic::<Marker>::init().expect("benches/bench.json should load");

    println!("{ROUNDS} reads per measurement\n");

    let plain = time("static ConfigCell", || {
        black_box(Plain::current().port);
    });

    let generic = time("registry (generic)", || {
        black_box(Generic::<Marker>::current().port);
    });

    println!("\ngeneric / static: {:.1}x", generic / plain);
}

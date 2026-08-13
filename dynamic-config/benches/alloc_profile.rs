//! The allocation profile of the read path, counted rather than assumed.
//!
//! The claim being audited: reading configuration allocates nothing —
//! `current()` is an atomic pointer load and an `Arc` clone on every
//! storage shape. A counting allocator makes that a number instead of a
//! sentence; if a future change puts an allocation on the read path, this
//! prints it.
//!
//! ```text
//! cargo bench -p dynamic-config --features json --bench alloc_profile
//! ```
//!
//! Not criterion: counting is exact, so statistics would only blur it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

use dynamic_config::{dynamic_config, Builder, Dynamic};
use serde::Deserialize;

/// The system allocator, with a tally.
///
/// This impl is the workspace's one audited `unsafe`: `GlobalAlloc` is an
/// unsafe trait and there is no safe way to hook the allocator. It never
/// ships — benches are their own crates, outside every published package —
/// and `security.yml`'s sweep names this file as the single exemption, so
/// a second `unsafe` anywhere in the test/bench/example tree is a red
/// gate, not a precedent.
struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

// SAFETY: delegates to `System` unchanged; the counter is the only addition.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Plain {
    port: u16,
}

#[derive(Debug, Deserialize)]
struct Instance {
    port: u16,
}

const ROUNDS: u64 = 100_000;

fn counted(label: &str, mut work: impl FnMut()) -> u64 {
    // Warm up outside the measurement: first calls may allocate lazily
    // (the registry's slot, a hook list), and steady state is the claim.
    for _ in 0..100 {
        work();
    }

    let before = ALLOCATIONS.load(Ordering::Relaxed);

    for _ in 0..ROUNDS {
        work();
    }

    let total = ALLOCATIONS.load(Ordering::Relaxed) - before;

    println!("{label:<24} {total:>6} allocations / {ROUNDS} reads");

    total
}

fn main() {
    Plain::builder("db")
        .file("benches/bench.json")
        .init()
        .expect("benches/bench.json should load");
    let dynamic = Dynamic::new(Builder::<Instance>::new("db").file("benches/bench.json"));
    dynamic.init().expect("benches/bench.json should load");

    println!("steady-state read path, counting allocator:\n");

    let a = counted("static current()", || {
        black_box(Plain::current().port);
    });
    let b = counted("Dynamic current()", || {
        black_box(dynamic.current().expect("initialised").port);
    });

    assert_eq!(
        (a, b),
        (0, 0),
        "the read path allocated — that is a regression, not a tuning knob"
    );

    println!("\nzero, on both shapes — the claim holds.\n");

    // A configuration with no struct reads by path, and the claim there is
    // the same one: the walk borrows out of the tree it already holds, so a
    // schemaless read allocates no more than a field access. What separates
    // `get` from `get_as` is *work*, not memory — measured in
    // `benches/read_path.rs`. The line memory does draw is what a read hands
    // back: a scalar copies out, an owned `String` cannot.
    let schemaless = Dynamic::new(dynamic_config::Builder::values("db").file("benches/bench.json"));
    schemaless.init().expect("benches/bench.json should load");

    let borrowed = counted("Value::get(path)", || {
        let values = schemaless.current().expect("initialised");

        black_box(
            values
                .get("pool.max_size")
                .and_then(dynamic_config::Value::as_i64),
        );
    });
    let scalar = counted("Value::get_as::<u16>", || {
        let values = schemaless.current().expect("initialised");

        black_box(values.get_as::<u16>("pool.max_size").expect("it is there"));
    });
    let owned = counted("Value::get_as::<String>", || {
        let values = schemaless.current().expect("initialised");

        black_box(values.get_as::<String>("host").expect("it is there"));
    });

    assert_eq!(
        (borrowed, scalar),
        (0, 0),
        "a path read borrows out of the installed tree, and rebuilding a \
         scalar to deserialize it touches no heap either; allocating means \
         one of those stopped being true"
    );
    assert!(
        owned > 0,
        "a read that hands back an owned String has to allocate it — if this \
         is ever zero the benchmark stopped measuring what it says"
    );

    println!(
        "\nreading by path allocates nothing, typed or not; what allocates is \
         handing back something owned — {} per `get_as::<String>`.",
        owned / ROUNDS
    );
}

//! Instruction counts for the four claims that are about *cost*.
//!
//! ```text
//! cargo bench -p dynamic-config --features json,toml --bench instructions
//! ```
//!
//! `read_path.rs` and `engine.rs` measure wall clock, which a shared runner
//! measures badly — which is why neither is a gate. Callgrind counts
//! instructions instead: the same number on the same binary every time, so a
//! change that adds an allocation to the read path shows up as a step rather
//! than as noise. That is the whole reason this file exists; anything whose
//! instruction count is not stable — the filesystem, the watcher, a remote
//! store — is deliberately absent.
//!
//! **Running this needs valgrind, and `iai-callgrind-runner` at exactly the
//! version of the `iai-callgrind` dev-dependency.** See CONTRIBUTING.md. The
//! ordinary gate only compiles this file (`cargo bench --no-run`), so a
//! benchmark that stopped building is caught without either.
//!
//! First measured 2026-08-13 and re-measured for 0.9.0 — rustc 1.97.1,
//! valgrind 3.24.0, glibc 2.43, x86_64 — so the bands below rest on a number
//! rather than on a guess:
//!
//! ```text
//!                        0.8.0      0.9.0
//! current_once              85         85 Ir
//! current_thousand      75,023     75,023 Ir   (75.0 per read)
//! load_one_document     20,942     15,021 Ir
//! reload_twenty_keys   183,791    132,402 Ir
//! explain_one_key       52,523     31,023 Ir
//! ```
//!
//! The load path moved twice in 0.9.0 and the second move is why the numbers
//! fell. Resolution stopped being one pass inside a backend and became a
//! pipeline — parse into this crate's tree, convert into the engine's,
//! fold, convert back, record provenance — which cost about twice a 0.8
//! load: 42,088 Ir through `config-rs` and 52,797 through figment, measured
//! here before the second move. Then `compose` stopped calling an engine for
//! a *single* layer, which is what a load with one file and no environment
//! actually is: a fold with nothing to merge is the layer, and the winner of
//! every leaf is the layer that supplied it. Both conversions and the merge
//! go, and what is left is cheaper than 0.8 was.
//!
//! Those are one machine's counts, not a baseline: the committed baseline has
//! to come off the CI image, because instruction counts differ by libc and
//! codegen (CONTRIBUTING.md). They are recorded here because they settle two
//! things the design could previously only assert. `setup` really is toggled
//! out of the measured region — installing a snapshot performs a load, a load
//! is 20,942 Ir, and `current_once` is 85, which could not contain one. And a
//! thousand reads cost a thousand times one read plus the loop, with two extra
//! RAM hits total, which is the no-allocation claim as arithmetic rather than
//! as prose.
//!
//! Each benchmark below runs in its own process, so the `static`s the macro
//! generates are not shared between them — but a config type is still declared
//! per benchmark rather than reused, because a future run under one process
//! would otherwise race in exactly the way `AGENTS.md` warns about.

use std::hint::black_box;

use dynamic_config::{dynamic_config, explain, load, Builder, Dynamic, Format, LoadSpec, Source};
use iai_callgrind::{
    library_benchmark, library_benchmark_group, main, Callgrind, EventKind, LibraryBenchmarkConfig,
};
use serde::Deserialize;

/// The thresholds, in the file rather than in a YAML string, because they are
/// a claim about the code and not about the runner.
///
/// A soft limit is a percentage against the committed baseline, and exceeding
/// one exits 3 — which is what makes the workflow a gate rather than a report.
/// Two numbers, not four: the read path should not move at all, while a reload
/// is allowed to drift with the loader it calls.
///
/// The bands are wider than measured noise, deliberately. Across five runs and
/// a rebuild, four of these five benchmarks reproduced to the instruction and
/// `reload_twenty_keys` moved at most 0.08 % — so 2 % is about twenty-five
/// times the spread anyone has actually seen. It stays that wide because the
/// spread that matters is the CI image's and nobody has measured that yet:
/// tightening to a laptop's figure is how a gate begins failing for reasons
/// nobody changed. Narrow it once a few green runs say what the runner's own
/// noise is.
fn within(percent: f64) -> LibraryBenchmarkConfig {
    let mut config = LibraryBenchmarkConfig::default();
    config.tool(Callgrind::default().soft_limits([(EventKind::Ir, percent)]));
    config
}

// ── The read path ──────────────────────────────────────────────────────

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Once {
    port: u16,
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Thousand {
    port: u16,
}

fn install_once(path: &str) {
    Once::builder("db")
        .file(path)
        .init()
        .expect("the bench fixture should load");
}

fn install_thousand(path: &str) {
    Thousand::builder("db")
        .file(path)
        .init()
        .expect("the bench fixture should load");
}

// **The claim: a read is an atomic load, not a parse.**
//
// Everything the crate says about being safe on a hot path reduces to this
// number staying small. `setup` installs the snapshot and callgrind toggles
// it out, so what is counted is the load and nothing before it.
#[library_benchmark(setup = install_once)]
#[bench::installed("benches/bench.json")]
fn current_once(_installed: ()) -> u16 {
    black_box(Once::current().port)
}

// **The claim: nothing is allocated per read.**
//
// An allocation on the read path is invisible in a single-read count — it is
// a handful of instructions against a hundred. A thousand reads make it
// arithmetic: with no per-read allocation this is a thousand times
// `current_once` plus the loop, and with one it is not.
#[library_benchmark(setup = install_thousand)]
#[bench::installed("benches/bench.json")]
fn current_thousand(_installed: ()) -> u16 {
    let mut last = 0;

    for _ in 0..1_000 {
        last = black_box(Thousand::current().port);
    }

    last
}

// ── The reload path ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Pool {
    #[allow(dead_code)]
    host: String,
    port: u16,
}

fn installed_pool(path: &str) -> Dynamic<Pool> {
    let config = Dynamic::new(Builder::<Pool>::new("db").file(path));
    config.init().expect("the bench fixture should load");
    config
}

// **The claim: the reload path has no accidental quadratic.**
//
// Twenty keys is a small service's configuration, and the count for it is
// only meaningful as a baseline — a reload that starts doing work per key
// per key moves this by a multiple, not by a percent, which is why its
// threshold is the loose one.
#[library_benchmark(setup = installed_pool)]
#[bench::twenty_keys("benches/twenty_keys.toml")]
fn reload_twenty_keys(config: Dynamic<Pool>) {
    config.reload().expect("the fixture has not changed");
}

// ── Diagnostics ────────────────────────────────────────────────────────

// **The claim: a diagnostic is not on any hot path.**
//
// `explain` re-walks every layer to say where a value came from, so this
// number is *expected* to be large. It is here as a ceiling: it is generous
// on purpose, and it exists so that "explain got slower" is a thing anybody
// notices before a user builds a dashboard out of it.
#[library_benchmark]
#[bench::one_key("benches/bench.json")]
fn explain_one_key(path: &str) {
    let sources = [Source::file(path, Format::Json)];
    let spec = LoadSpec::new("db", &sources);

    black_box(explain(&spec, "pool.max_size").expect("the key is in the fixture"));
}

// **The claim underneath the other three: a load is one parse.**
//
// `load` is what a reload calls once the sources are decided, measured
// without the cell, the hooks or the generation counter around it — so a
// regression here says the loader moved, and one in `reload_twenty_keys`
// alone says the machinery around it did.
#[library_benchmark]
#[bench::one_document("benches/bench.json")]
fn load_one_document(path: &str) -> u16 {
    let sources = [Source::file(path, Format::Json)];

    black_box(
        load::<Pool>(&LoadSpec::new("db", &sources))
            .expect("the bench fixture should load")
            .port,
    )
}

library_benchmark_group!(
    name = read_path;
    config = within(2.0);
    benchmarks = current_once, current_thousand
);

library_benchmark_group!(
    name = reload;
    config = within(10.0);
    benchmarks = reload_twenty_keys, load_one_document
);

// A ceiling rather than a measurement: `explain` is allowed to be expensive,
// and this exists so that "expensive" cannot quietly become "quadratic".
library_benchmark_group!(
    name = diagnostics;
    config = within(25.0);
    benchmarks = explain_one_key
);

main!(library_benchmark_groups = read_path, reload, diagnostics);

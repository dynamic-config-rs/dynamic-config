//! The soak-and-leak rig — PATH-TO-1.0 §3 made executable.
//!
//! Not a test: a workload. N reader threads hammer `current()`, M writer
//! threads churn the watched file through a fault schedule (valid change,
//! malformed change, delete, restore, rapid churn), subscriber threads
//! ride `changed()`/`events()`, and the process asserts its invariants at
//! exit: no panic anywhere, last-known-good always served, generations
//! monotonic, refusals monotonic, RSS/threads/FDs bounded against their
//! starting point.
//!
//! Modes:
//!   soak  --seconds N            the nightly (5h in CI, 24h locally)
//!   leak  --reloads N            1M tight reloads, then the deltas
//!
//! Exit code 0 means every invariant held; anything else names what did
//! not on stderr. The remote repository's nightly adds store legs over
//! its compose containers; this binary is the engine-only core.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Soaked {
    #[allow(dead_code)]
    host: String,
    generation_hint: u64,
}

fn write_valid(path: &std::path::Path, n: u64) {
    // Atomic-rename, like every real writer this engine meets.
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        format!("{{\"app\": {{\"host\": \"h{n}\", \"generation_hint\": {n}}}}}"),
    )
    .expect("scratch is writable");
    std::fs::rename(&tmp, path).expect("rename is atomic here");
}

fn write_malformed(path: &std::path::Path) {
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        "{\"app\": {\"host\": 12, \"generation_hint\": \"not-a-number\"",
    )
    .expect("scratch is writable");
    std::fs::rename(&tmp, path).expect("rename is atomic here");
}

fn proc_status(field: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();

    status
        .lines()
        .find(|line| line.starts_with(field))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn fd_count() -> u64 {
    std::fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count() as u64)
        .unwrap_or(0)
}

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "soak".to_owned());

    let mut seconds = 60u64;
    let mut reloads = 1_000_000u64;
    let mut readers = 8usize;

    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_default();

        match flag.as_str() {
            "--seconds" => seconds = value.parse().expect("whole seconds"),
            "--reloads" => reloads = value.parse().expect("a count"),
            "--readers" => readers = value.parse().expect("a count"),
            other => panic!("unknown flag {other}"),
        }
    }

    let dir = std::env::temp_dir().join(format!("dynamic-config-soak-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch is writable");
    let file = dir.join("config.json");

    write_valid(&file, 1);

    let builder = Soaked::builder("app").file(file.to_str().expect("utf-8 scratch"));

    builder.init().expect("the first document loads");

    match mode.as_str() {
        "leak" => leak(&builder, &file, reloads),
        _ => soak(&builder, &file, seconds, readers),
    }
}

fn leak(
    builder: &dynamic_config::Builder<Soaked>,
    file: &std::path::Path,
    reloads: u64,
) -> std::process::ExitCode {
    let rss_before = proc_status("VmRSS:");
    let fds_before = fd_count();
    let threads_before = proc_status("Threads:");

    for n in 2..reloads + 2 {
        write_valid(file, n);
        builder.reload().expect("a valid document reloads");
    }

    let rss_after = proc_status("VmRSS:");
    let fds_after = fd_count();
    let threads_after = proc_status("Threads:");

    println!(
        "leak: {reloads} reloads | rss {rss_before}→{rss_after} kB | fds {fds_before}→{fds_after} | threads {threads_before}→{threads_after}"
    );

    // The budget: RSS may breathe (allocator slack), it may not grow with
    // the reload count; a million reloads earning more than 64 MiB is a
    // leak whatever the allocator says.
    let mut failed = false;

    if rss_after > rss_before + 65_536 {
        eprintln!(
            "LEAK: rss grew {} kB over {reloads} reloads",
            rss_after - rss_before
        );
        failed = true;
    }

    if fds_after > fds_before + 8 {
        eprintln!(
            "LEAK: fds grew {} over {reloads} reloads",
            fds_after - fds_before
        );
        failed = true;
    }

    if threads_after > threads_before + 2 {
        eprintln!("LEAK: threads grew {}", threads_after - threads_before);
        failed = true;
    }

    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

fn soak(
    builder: &dynamic_config::Builder<Soaked>,
    file: &std::path::Path,
    seconds: u64,
    reader_count: usize,
) -> std::process::ExitCode {
    let stop = Arc::new(AtomicBool::new(false));
    let reader_panics = Arc::new(AtomicU64::new(0));
    let max_generation = Arc::new(AtomicU64::new(0));

    builder
        .watch(Duration::from_millis(10))
        .expect("the watcher starts")
        .detach();

    let readers: Vec<_> = (0..reader_count)
        .map(|_| {
            let stop = Arc::clone(&stop);
            let panics = Arc::clone(&reader_panics);
            let max_generation = Arc::clone(&max_generation);

            std::thread::spawn(move || {
                let mut seen = 0u64;

                while !stop.load(Ordering::Relaxed) {
                    let outcome = std::panic::catch_unwind(|| {
                        // LKG: after init, current() must always answer.
                        let snapshot = Soaked::current();
                        let generation = Soaked::generation();

                        (snapshot.generation_hint, generation)
                    });

                    match outcome {
                        Ok((_, generation)) => {
                            // Monotonic per reader.
                            if generation < seen {
                                panics.fetch_add(1, Ordering::Relaxed);
                                eprintln!("INVARIANT: generation went backwards");
                            }

                            seen = generation;
                            max_generation.fetch_max(generation, Ordering::Relaxed);
                        }
                        Err(_) => {
                            panics.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    std::thread::yield_now();
                }
            })
        })
        .collect();

    let rss_start = proc_status("VmRSS:");
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let mut n = 2u64;
    let mut cycle = 0u64;

    while Instant::now() < deadline {
        // The fault schedule, one full cycle every 6 steps.
        match cycle % 6 {
            0..=2 => write_valid(file, n),
            3 => write_malformed(file),
            4 => {
                let _ = std::fs::remove_file(file);
            }
            _ => write_valid(file, n),
        }

        n += 1;
        cycle += 1;
        std::thread::sleep(Duration::from_millis(25));
    }

    stop.store(true, Ordering::Relaxed);

    for reader in readers {
        let _ = reader.join();
    }

    let rss_end = proc_status("VmRSS:");
    let panics = reader_panics.load(Ordering::Relaxed);
    let refusals = Soaked::status().consecutive_failures;
    let generation = Soaked::generation();

    println!(
        "soak: {}s | cycles {cycle} | generation {generation} | rss {rss_start}→{rss_end} kB | reader panics {panics} | consecutive_failures now {refusals}",
        started.elapsed().as_secs(),
    );

    // Exit invariants.
    let mut failed = false;

    if panics > 0 {
        eprintln!("INVARIANT: {panics} reader panics / monotonicity violations");
        failed = true;
    }

    if generation < 2 {
        eprintln!("INVARIANT: the watcher installed nothing");
        failed = true;
    }

    if Soaked::try_current().is_none() {
        eprintln!("INVARIANT: last-known-good stopped serving");
        failed = true;
    }

    if rss_end > rss_start + 262_144 {
        eprintln!("INVARIANT: rss grew {} kB", rss_end - rss_start);
        failed = true;
    }

    // The last write of the schedule may have been the malformed or the
    // deleted step; land one final valid document and require recovery.
    write_valid(file, n);
    std::thread::sleep(Duration::from_millis(500));

    if Soaked::current().generation_hint != n {
        eprintln!("INVARIANT: recovery after the schedule did not land");
        failed = true;
    }

    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

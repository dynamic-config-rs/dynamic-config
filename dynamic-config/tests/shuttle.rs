//! Randomised, replayable scheduling with shuttle: the residue loom cannot
//! reach.
//!
//! Runs only under `RUSTFLAGS="--cfg shuttle"` (see `just shuttle`), where
//! `src/sync.rs` hands the library shuttle's primitives instead of `std`'s —
//! so these models drive the *real* code, exactly as the loom suite does.
//!
//! # Why both
//!
//! `tests/loom.rs` is exhaustive and therefore small: it proves the remote
//! fence and `Notify::poll_with` over *every* interleaving, and it models
//! atomic orderings faithfully, so a `Relaxed` that should have been
//! `Acquire` fails there. What it cannot do is run any of this:
//!
//! * **`arc-swap`.** loom cannot instrument it, so `ConfigCell` — whose
//!   whole read-path argument is an `ArcSwap` — has no loom model at all.
//!   Shuttle runs it. What shuttle does *not* do is see inside it either:
//!   arc-swap's own atomics are `std`'s, so shuttle places no yieldpoint
//!   within a `load`/`swap`/`rcu`. Those operations therefore execute
//!   atomically under shuttle, and these models prove the *composition*
//!   around them, not arc-swap's internals. Said plainly: shuttle cannot
//!   observe a torn read here, so "no torn read" is not what is being
//!   claimed — the claims below are about generations, hook lists and
//!   wake-ups, which are ours.
//! * **process-wide `static`s.** loom's iteration model wants all state
//!   reachable from the model, and a `static` that survives one iteration
//!   poisons the next. `shuttle::lazy_static!` re-initialises per execution,
//!   which is what lets the `ConfigCell`-in-a-`static` shape — the shape
//!   `#[dynamic_config]` actually emits — be modelled at all.
//! * **anything with a state space this size.** Three threads through
//!   `ReloadGroup::reload`, each running a mutex acquire and three commits,
//!   is not an exhaustive-search-sized problem.
//!
//! The trade is soundness: a passing shuttle run proves nothing, it only
//! fails to disprove. Both suites stay.
//!
//! # Reproducing a failure
//!
//! Every model runs from a fixed seed by default, so this suite is a
//! regression test rather than a search and CI can gate on it without being
//! flaky. Two knobs, both printed at the top of each model:
//!
//! ```text
//! SHUTTLE_SEED=12345 just shuttle      # a different fixed seed
//! SHUTTLE_SEED=random just shuttle     # search; the drawn seed is printed
//! SHUTTLE_ITERATIONS=100000 just shuttle
//! ```
//!
//! On failure shuttle also prints the exact schedule, which
//! `shuttle::replay(body, "…")` re-runs step for step.

#![cfg(shuttle)]

use std::sync::Arc;

use dynamic_config::{Commit, ConfigCell, Error, ReloadGroup, Reloadable};

/// Measured rather than guessed: one execution of these models costs about
/// 22 µs in a release build, so all four together check 200 000 schedules in
/// a little over a second and the CI job's cost is compiling, not checking.
/// Raise it with `SHUTTLE_ITERATIONS` when hunting something.
const DEFAULT_ITERATIONS: usize = 50_000;

/// Arbitrary, and that is the point: a constant seed makes every run explore
/// the *same* schedules, which is what makes this gateable.
const DEFAULT_SEED: u64 = 0x64_79_6e_61_6d_69_63;

/// Runs one model, announcing how to reproduce whatever it does.
///
/// `SHUTTLE_SEED` is this harness's, not shuttle's. Shuttle's own
/// `SHUTTLE_RANDOM_SEED` only reaches `check_random`, and these models call
/// `check_random_with_seed` so that the default run is deterministic —
/// setting shuttle's variable here does nothing.
fn model(name: &str, body: impl Fn() + Send + Sync + 'static) {
    let iterations = std::env::var("SHUTTLE_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS);

    // A drawn seed is printed rather than left implicit: an unreplayable
    // concurrency failure is noise, and `check_random` alone would leave the
    // seed inside the scheduler.
    let seed = match std::env::var("SHUTTLE_SEED").as_deref() {
        Ok("random") => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(DEFAULT_SEED, |since| since.as_nanos() as u64),
        Ok(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("SHUTTLE_SEED must be a u64 or `random`, got {value:?}")),
        Err(_) => DEFAULT_SEED,
    };

    eprintln!("shuttle: {name} — {iterations} schedules, seed {seed}");
    eprintln!("         reproduce with SHUTTLE_SEED={seed} SHUTTLE_ITERATIONS={iterations}");

    shuttle::check_random_with_seed(body, seed, iterations);
}

// ---------------------------------------------------------------------------
// `ConfigCell`: the install path, which has no loom model at all
// ---------------------------------------------------------------------------

/// Covers `src/cell.rs`: `store`, `load`, `generation`, `meta`.
///
/// Two threads installing into a cold cell — what two `init()` calls on one
/// type reduce to once the loading is done. The claims:
///
/// * the `OnceLock` race leaves exactly one live cell, so the generation
///   counter counts *both* installs rather than one per winner;
/// * `meta` never leads the value: whatever a reader loads, it was stored by
///   somebody, and the generation it reports is one of the installs that
///   really happened;
/// * a reader running alongside never observes a value nobody wrote.
#[test]
fn two_installs_into_a_cold_cell_agree_on_one_cell() {
    model("two_installs_into_a_cold_cell_agree_on_one_cell", || {
        let cell: Arc<ConfigCell<u64>> = Arc::new(ConfigCell::new());

        let writers: Vec<_> = [1u64, 2]
            .into_iter()
            .map(|value| {
                let cell = Arc::clone(&cell);

                shuttle::thread::spawn(move || cell.store(value))
            })
            .collect();

        let reader = {
            let cell = Arc::clone(&cell);

            shuttle::thread::spawn(move || {
                if let Some(seen) = cell.load() {
                    assert!(
                        *seen == 1 || *seen == 2,
                        "a reader observed {seen}, which nobody stored"
                    );
                }
            })
        };

        for writer in writers {
            writer.join().unwrap();
        }
        reader.join().unwrap();

        assert_eq!(
            cell.generation(),
            2,
            "both installs must be counted: a lost one means the `OnceLock` \
             race produced two cells, or `meta.rcu` dropped an update"
        );
        let installed = *cell.load().expect("two threads stored");
        assert!(installed == 1 || installed == 2);
        assert_eq!(
            cell.meta().expect("two threads stored").generation,
            2,
            "metadata must not lag the last install once every writer has joined"
        );
    });
}

// ---------------------------------------------------------------------------
// The hook list: register and unregister racing a reload
// ---------------------------------------------------------------------------

shuttle::lazy_static! {
    /// The cell model 2 registers against. A `static`, because
    /// `on_reload_scoped` takes `&'static self` — and because a `static` is
    /// what `#[dynamic_config]` emits. `shuttle::lazy_static!` gives a fresh
    /// one per execution, which is precisely what loom cannot do.
    static ref HOOKED: ConfigCell<u64> = ConfigCell::new();
}

/// Covers `src/cell.rs`: `register`, `unregister`, `dispatch`, `HookGuard::drop`.
///
/// Three threads each register a scoped hook, reload, then drop the guard,
/// all overlapping. Both halves of the `rcu` pair are on trial:
///
/// * **no lost registration** — every thread's own reload happens while its
///   own guard is alive, so at least three dispatches must be observed; a
///   `register` whose rebuild was clobbered by a concurrent `unregister`
///   loses one;
/// * **no lost unregistration** — once every guard is dropped, a further
///   reload must dispatch nothing at all; an `unregister` clobbered by a
///   concurrent `register` leaves a torn-down hook firing forever.
#[test]
fn hooks_registered_and_dropped_concurrently_are_neither_lost_nor_leaked() {
    model(
        "hooks_registered_and_dropped_concurrently_are_neither_lost_nor_leaked",
        || {
            let cell: &'static ConfigCell<u64> = &HOOKED;

            // The first store is an initialization, not a reload; from here
            // on every store dispatches.
            cell.store(0);

            let fired = Arc::new(shuttle::sync::atomic::AtomicUsize::new(0));

            let threads: Vec<_> = (1..=3u64)
                .map(|value| {
                    let fired = Arc::clone(&fired);

                    shuttle::thread::spawn(move || {
                        let guard = cell.on_reload_scoped(move |_, _| {
                            fired.fetch_add(1, shuttle::sync::atomic::Ordering::SeqCst);
                        });

                        cell.store(value);
                        drop(guard);
                    })
                })
                .collect();

            for thread in threads {
                thread.join().unwrap();
            }

            let during = fired.load(shuttle::sync::atomic::Ordering::SeqCst);
            assert!(
                (3..=9).contains(&during),
                "three reloads against at most three live hooks, each thread \
                 seeing at least its own: {during} dispatches"
            );

            fired.store(0, shuttle::sync::atomic::Ordering::SeqCst);
            cell.store(99);
            assert_eq!(
                fired.load(shuttle::sync::atomic::Ordering::SeqCst),
                0,
                "every guard was dropped, so no hook may still be registered"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// `ReloadGroup`: the commit block, under concurrent reloaders
// ---------------------------------------------------------------------------

shuttle::lazy_static! {
    /// What each member's commit appends. Behind shuttle's mutex rather than
    /// `std`'s: a `std` lock that actually contended inside a shuttle
    /// execution would block the one OS thread every task shares.
    static ref COMMITTED: shuttle::sync::Mutex<Vec<&'static str>> =
        shuttle::sync::Mutex::new(Vec::new());
}

/// Declares a `Reloadable` whose commit appends its own name.
macro_rules! member {
    ($name:ident) => {
        struct $name;

        impl Reloadable for $name {
            fn prepare() -> Result<Commit, Error> {
                Ok(Box::new(|| {
                    COMMITTED
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(stringify!($name));
                }))
            }

            fn name() -> &'static str {
                stringify!($name)
            }
        }
    };
}

member!(Alpha);
member!(Beta);
member!(Gamma);
member!(Delta);

/// The member that fails in `prepare`, so its group must commit nothing.
struct Broken;

impl Reloadable for Broken {
    fn prepare() -> Result<Commit, Error> {
        Err(Error::invalid("nothing supplies it"))
    }

    fn name() -> &'static str {
        "Broken"
    }
}

/// Covers `src/group.rs`: `ReloadGroup::reload` and the `reloading` mutex.
///
/// Two threads reload the same group while a third reloads a group that
/// fails. The claims:
///
/// * **a commit loop is a block.** The mutex is what makes it one, and the
///   failure it prevents is member A on one reloader's snapshot and member B
///   on the other's — so the log must be two intact `Alpha, Beta, Gamma`
///   runs, never an interleaving of them;
/// * **all or nothing.** `Delta` prepares cleanly and `Broken` does not, so
///   `Delta` must never appear: dropping the collected commits is what "not
///   applied" means, and a third reloader running concurrently must not
///   change that.
///
/// loom cannot run this one: three threads, a mutex, six commits and a
/// `lazy_static` log is not an exhaustive-search-sized state space.
#[test]
fn concurrent_group_reloads_commit_as_blocks_and_a_failing_group_commits_nothing() {
    model(
        "concurrent_group_reloads_commit_as_blocks_and_a_failing_group_commits_nothing",
        || {
            let healthy = Arc::new(
                ReloadGroup::new()
                    .with::<Alpha>()
                    .with::<Beta>()
                    .with::<Gamma>(),
            );
            let doomed = Arc::new(ReloadGroup::new().with::<Delta>().with::<Broken>());

            let reloaders: Vec<_> = (0..2)
                .map(|_| {
                    let healthy = Arc::clone(&healthy);

                    shuttle::thread::spawn(move || healthy.reload().expect("every member prepares"))
                })
                .collect();

            let failing =
                shuttle::thread::spawn(move || doomed.reload().expect_err("Broken never prepares"));

            for reloader in reloaders {
                reloader.join().unwrap();
            }
            let error = failing.join().unwrap();
            assert!(error.path().starts_with("Broken"), "{error}");

            let log = COMMITTED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();

            assert_eq!(
                log,
                ["Alpha", "Beta", "Gamma", "Alpha", "Beta", "Gamma"],
                "two reloads of one group must be two intact blocks, and the \
                 failing group must have committed nothing"
            );
        },
    );
}

// ---------------------------------------------------------------------------
// The wake protocol, end to end
// ---------------------------------------------------------------------------

shuttle::lazy_static! {
    /// The cell model 4 awaits. Also a `static`, for the same reason:
    /// `changes()` takes `&'static self`.
    static ref AWAITED: ConfigCell<u64> = ConfigCell::new();
}

/// Covers `src/cell.rs::store` composed with `src/asynchronous.rs`:
/// `Notify::bump`, `Notify::poll_with`, `Changes::changed`.
///
/// loom drives `poll_with` directly, against a synthetic load closure, because
/// the real one goes through `arc-swap`. This model closes that gap: a real
/// `store` — the `ArcSwap::swap`, the `meta.rcu`, the `bump`, in that order —
/// against a real `Changes` awaiting it.
///
/// The claims:
///
/// * **no lost wake-up.** A bump landing anywhere inside check-register-check
///   still completes the future. Shuttle reports a stalled execution as a
///   deadlock, so a lost wake-up fails the model rather than hanging CI.
/// * **the value a wake implies is there.** `bump` happens *after* the swap,
///   so a woken waiter that loads must not see the pre-store snapshot.
#[test]
fn a_waiter_always_wakes_and_never_wakes_to_the_old_snapshot() {
    model(
        "a_waiter_always_wakes_and_never_wakes_to_the_old_snapshot",
        || {
            let cell: &'static ConfigCell<u64> = &AWAITED;

            cell.store(1);

            // Created before the writers start, so `seen` is the generation of
            // the baseline install and every later store is a change.
            let mut changes = cell.changes();

            let writers: Vec<_> = [2u64, 3]
                .into_iter()
                .map(|value| shuttle::thread::spawn(move || cell.store(value)))
                .collect();

            let observed = shuttle::future::block_on(async { *changes.changed().await });

            assert!(
                observed == 2 || observed == 3,
                "a waiter woken by a store must not see the snapshot that store \
             replaced; saw {observed}"
            );

            for writer in writers {
                writer.join().unwrap();
            }
        },
    );
}

//! What a reload hook is promised when two reloads overlap.
//!
//! Reloads are deliberately not serialised: the store wakes async waiters
//! before it runs hooks, so that one slow callback cannot delay every
//! reader, and a lock held across user callbacks would undo that. What is
//! left is a contract worth stating precisely, and this is it stated as a
//! test — a *consistent pair* every call, and no claim about order.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use dynamic_config::ConfigCell;

/// Every `(previous, current)` a hook is handed is two adjacent installs:
/// both happened, and `current` replaced `previous`.
///
/// Asserted by reconstructing the install order *from the pairs* — if each
/// value is `previous` for exactly one pair and `current` for exactly one
/// pair, the pairs link into a single chain, and being adjacent in that
/// chain is the whole of the guarantee. Nothing here says anything about
/// the order the hook was called in, because the documentation says there
/// is none.
#[test]
fn every_pair_a_hook_sees_is_two_adjacent_installs() {
    const WRITERS: u64 = 2;
    const PER_WRITER: u64 = 500;

    let cell: &'static ConfigCell<u64> = Box::leak(Box::new(ConfigCell::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));

    {
        let recorder = Arc::clone(&seen);
        cell.on_reload(move |previous, current| {
            recorder.lock().unwrap().push((**previous, **current));
        });
    }

    // Installed before the writers start, so the chain has one head: on a
    // cold cell both writers' first store would dispatch.
    cell.store(0);

    let writers: Vec<_> = (1..=WRITERS)
        .map(|writer| {
            thread::spawn(move || {
                for step in 1..=PER_WRITER {
                    cell.store(writer * PER_WRITER + step);
                }
            })
        })
        .collect();

    for writer in writers {
        writer.join().unwrap();
    }

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.len() as u64,
        WRITERS * PER_WRITER,
        "one dispatch per install, whoever won which race"
    );

    let mut successor = HashMap::new();

    for (previous, current) in seen.iter().copied() {
        assert_ne!(
            previous, current,
            "a hook is never handed one snapshot twice"
        );
        assert!(
            successor.insert(previous, current).is_none(),
            "{previous} was replaced by two different snapshots"
        );
    }

    let mut installed = 0;
    for _ in 0..seen.len() {
        installed = *successor
            .get(&installed)
            .unwrap_or_else(|| panic!("the chain of pairs breaks at {installed}"));
    }

    assert!(
        !successor.contains_key(&installed),
        "the chain must end where the installs did"
    );
    assert_eq!(
        *cell.load().expect("something is installed"),
        installed,
        "and what it ends at is what is serving"
    );
}

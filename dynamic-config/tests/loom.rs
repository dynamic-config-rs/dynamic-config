//! Model checking, with loom: every interleaving of the claims we make.
//!
//! Runs only under `RUSTFLAGS="--cfg loom"` (see `just loom`), where the
//! library's sync primitives are loom's — the models below drive the *real*
//! code, not a copy. `ConfigCell`'s swap is deliberately absent: it lives
//! inside `arc-swap`, which loom cannot instrument, and its wake protocol —
//! the part that is ours — is checked through `Notify::poll_with`.

#![cfg(loom)]

use dynamic_config::{Error, Fetched, Format, Remote, RemoteSource};

struct Fake(&'static str);

impl RemoteSource for Fake {
    fn describe(&self) -> String {
        "a fake".to_owned()
    }

    fn fetch(&self) -> Result<Fetched, Error> {
        Ok(Fetched::new(self.0, Format::Json))
    }
}

const DOC_A: &str = r#"{"db": {"host": "a"}}"#;
const DOC_B: &str = r#"{"db": {"host": "b"}}"#;

/// The fetch fence: a refresh racing a source swap never leaves the old
/// source's document installed.
#[test]
fn a_refresh_racing_a_swap_never_installs_the_old_document() {
    loom::model(|| {
        let remote = loom::sync::Arc::new(Remote::new());
        remote.set(Fake(DOC_A));

        let fetcher = {
            let remote = loom::sync::Arc::clone(&remote);
            loom::thread::spawn(move || {
                // May fail benignly mid-swap ("no source" is unreachable
                // here, but a discarded commit is quiet success).
                let _ = remote.refresh();
            })
        };
        let swapper = {
            let remote = loom::sync::Arc::clone(&remote);
            loom::thread::spawn(move || {
                remote.set(Fake(DOC_B));
            })
        };

        fetcher.join().unwrap();
        swapper.join().unwrap();

        // Every interleaving ends with either nothing (the fetch was
        // overtaken, or the swap cleared it) or the new source's document —
        // never A's. And when nothing survived, the slot must still be
        // alive: a refresh against the new source has to work, or the
        // fence would be a brick rather than a filter.
        match remote.document() {
            Some(document) => {
                assert_eq!(
                    document.text, DOC_B,
                    "the old source's fetch leaked through"
                );
            }
            None => {
                remote.refresh().expect("the new source still fetches");
                assert_eq!(
                    remote.document().expect("just fetched").text,
                    DOC_B,
                    "a refresh after the race must serve the new source"
                );
            }
        }
    });
}

/// The push fence: a generation captured before a swap can never install
/// afterwards, whichever way the two interleave.
#[test]
fn a_push_with_a_pre_swap_generation_never_lands() {
    loom::model(|| {
        let remote = loom::sync::Arc::new(Remote::new());
        remote.set(Fake(DOC_A));
        let stale = remote.generation_for_loom();

        let pusher = {
            let remote = loom::sync::Arc::clone(&remote);
            loom::thread::spawn(move || {
                let _ = remote.install_if_for_loom(stale, Fetched::new(DOC_A, Format::Json));
            })
        };
        let swapper = {
            let remote = loom::sync::Arc::clone(&remote);
            loom::thread::spawn(move || {
                remote.set(Fake(DOC_B));
            })
        };

        pusher.join().unwrap();
        swapper.join().unwrap();

        // The swap always runs, so the captured generation is always stale
        // by the end: install-then-swap is cleared by the swap, swap-then-
        // install bounces. Either way, nothing survives.
        assert!(
            remote.document().is_none(),
            "a stale push survived an interleaving"
        );
    });
}

/// The wake protocol: a bump between the check and the registration is not
/// a lost wake-up. loom fails the model if the future can never complete.
#[test]
fn a_bump_racing_the_check_register_check_is_never_lost() {
    use std::future::poll_fn;

    loom::model(|| {
        let notify = loom::sync::Arc::new(dynamic_config::LoomNotify::new());
        let value = loom::sync::Arc::new(loom::sync::atomic::AtomicBool::new(false));

        let bumper = {
            let notify = loom::sync::Arc::clone(&notify);
            let value = loom::sync::Arc::clone(&value);
            loom::thread::spawn(move || {
                value.store(true, loom::sync::atomic::Ordering::Release);
                notify.bump();
            })
        };

        let mut seen = 0u64;
        loom::future::block_on(poll_fn(|context| {
            notify.poll_with(&mut seen, context.waker(), || {
                value
                    .load(loom::sync::atomic::Ordering::Acquire)
                    .then_some(())
            })
        }));

        bumper.join().unwrap();
    });
}

//! Configuration fetched from somewhere else, merged like a file.
//!
//! No network here: the sources are in-process fakes. What the companion
//! crates add is a client; the layering, the precedence and the timing are all
//! decided here, and this is where they are pinned.

#![cfg(feature = "json")]

use std::sync::atomic::{AtomicUsize, Ordering};

use dynamic_config::{dynamic_config, Error, Fetched, Format, Origin, RemoteSource};
use serde::Deserialize;

macro_rules! db_config {
    ($name:ident) => {
        #[dynamic_config]
        #[derive(Debug, Deserialize)]
        struct $name {
            // Read through `load()`'s success rather than by name: these
            // tests are about which layer supplies a value, not about the
            // struct that receives it.
            #[allow(dead_code)]
            host: String,
            #[allow(dead_code)]
            port: u16,
        }

        impl $name {
            /// The `db` section of the base fixture, overridable through
            /// `DCREMOTE_DB_*`.
            // Not every generated type loads; a refusal needs no sources.
            #[allow(dead_code)]
            fn sources() -> dynamic_config::Builder<$name> {
                $name::builder("db")
                    .file("tests/fixtures/base.json")
                    .env("DCREMOTE_")
            }
        }
    };
}

db_config!(Lazy);
db_config!(Layered);
db_config!(Failing);

/// Counts its calls, so "fetched once" can be told from "fetched per load".
struct Counting {
    document: &'static str,
    calls: &'static AtomicUsize,
}

impl RemoteSource for Counting {
    fn fetch(&self) -> Result<Fetched, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);

        Ok(Fetched::new(self.document, Format::Json))
    }

    fn describe(&self) -> String {
        "a counting store".to_owned()
    }
}

struct Unreachable;

impl RemoteSource for Unreachable {
    fn fetch(&self) -> Result<Fetched, Error> {
        Err(Error::remote("connection refused"))
    }

    fn describe(&self) -> String {
        "an unreachable store".to_owned()
    }
}

// A counter per test: these run in parallel and would otherwise count each
// other's fetches.
static LAZY_CALLS: AtomicUsize = AtomicUsize::new(0);
static LAYERED_CALLS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn loading_never_touches_the_network() {
    Lazy::set_remote(Counting {
        document: r#"{"db": {"port": 9000}}"#,
        calls: &LAZY_CALLS,
    });

    assert_eq!(
        LAZY_CALLS.load(Ordering::SeqCst),
        0,
        "installing must not fetch"
    );

    // Loads before the first refresh see no remote layer at all.
    assert_eq!(Lazy::sources().load().unwrap().port, 5432);
    assert_eq!(LAZY_CALLS.load(Ordering::SeqCst), 0);

    Lazy::refresh_remote().unwrap();
    assert_eq!(LAZY_CALLS.load(Ordering::SeqCst), 1);

    // ...and every load after it reads the kept document, not the store.
    for _ in 0..5 {
        assert_eq!(Lazy::sources().load().unwrap().port, 9000);
    }

    assert_eq!(
        LAZY_CALLS.load(Ordering::SeqCst),
        1,
        "a round trip per configuration read would be indefensible"
    );

    Lazy::clear_remote();
}

#[test]
fn a_remote_document_beats_a_file_and_loses_to_the_environment() {
    Layered::set_remote(Counting {
        document: r#"{"db": {"host": "from-remote", "port": 9000}}"#,
        calls: &LAYERED_CALLS,
    });
    Layered::refresh_remote().unwrap();

    let config = Layered::sources().load().unwrap();
    assert_eq!(config.host, "from-remote", "the remote beats the file");
    assert_eq!(
        Layered::sources().source_of("host").unwrap(),
        Some(Origin::Remote("a counting store".to_owned())),
        "the store's own `describe` is what a traced value names — not \
         \"an inline source\", which is what figment sees and is the wrong \
         answer to the question being asked"
    );

    // The machine's own environment still wins.
    std::env::set_var("DCREMOTE_DB_HOST", "from-the-machine");

    assert_eq!(Layered::sources().load().unwrap().host, "from-the-machine");

    std::env::remove_var("DCREMOTE_DB_HOST");
    Layered::clear_remote();
}

#[test]
fn an_unreachable_store_reports_and_leaves_the_files_serving() {
    Failing::set_remote(Unreachable);

    let error = Failing::refresh_remote().expect_err("the store is unreachable");
    assert_eq!(error.kind(), dynamic_config::ErrorKind::Remote);

    // The failure is in the refresh, not the load: a store that is down does
    // not stop a program that has files.
    assert_eq!(Failing::sources().load().unwrap().port, 5432);

    Failing::clear_remote();
}

#[cfg(feature = "async")]
mod asynchronous {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;

    db_config!(Streamed);

    struct Streaming;

    impl dynamic_config::AsyncRemoteSource for Streaming {
        fn fetch(&self) -> Pin<Box<dyn Future<Output = Result<Fetched, Error>> + Send + '_>> {
            Box::pin(async { Ok(Fetched::new(r#"{"db": {"port": 7000}}"#, Format::Json)) })
        }

        fn describe(&self) -> String {
            "a streaming store".to_owned()
        }
    }

    #[tokio::test]
    async fn an_async_source_reaches_the_same_layer() {
        Streamed::set_remote_async(Streaming);
        Streamed::refresh_remote_async().await.unwrap();

        assert_eq!(Streamed::sources().load().unwrap().port, 7000);

        Streamed::clear_remote();
    }

    #[tokio::test]
    async fn refreshing_an_async_source_synchronously_says_which_call_to_use() {
        db_config!(Confused);

        Confused::set_remote_async(Streaming);

        let error = Confused::refresh_remote().expect_err("this one is async");

        assert!(
            error.to_string().contains("refresh_remote_async"),
            "{error}"
        );

        Confused::clear_remote();
    }
}

// ---------------------------------------------------------------------------
// What a watch pushes through
// ---------------------------------------------------------------------------

mod watching {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use dynamic_config::{dynamic_config, Fetched, Format, Origin, RemoteWatch, Watching};
    use serde::Deserialize;

    #[dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Pushed {
        #[allow(dead_code)]
        host: String,
        #[allow(dead_code)]
        port: u16,
    }

    static RELOADS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn a_pushed_document_reloads_the_snapshot() {
        Pushed::on_reload(|_previous, _current| {
            RELOADS.fetch_add(1, Ordering::SeqCst);
        });

        // `remote_sink().apply(..)` reloads through the builder the type was configured
        // with, so the `init` below is what gives the push somewhere to land.
        Pushed::builder("db")
            .file("tests/fixtures/base.json")
            .init()
            .expect("the file alone is a whole configuration");
        assert_eq!(
            Pushed::current().port,
            5432,
            "the file's value, to begin with"
        );

        Pushed::remote_sink()
            .apply(Fetched::new(r#"{"db": {"port": 9100}}"#, Format::Json))
            .expect("the document merges over the file");

        assert_eq!(Pushed::current().port, 9100);
        assert_eq!(
            Pushed::current().host,
            "localhost",
            "a partial document merges rather than replacing"
        );
        assert_eq!(
            RELOADS.load(Ordering::SeqCst),
            1,
            "a push must run the reload hooks, exactly as a file edit does"
        );

        // And it is traced to the store, not to an inline source.
        assert!(
            matches!(Pushed::source_of("port").unwrap(), Some(Origin::Remote(_))),
            "{:?}",
            Pushed::source_of("port").unwrap()
        );
    }

    #[dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Rejecting {
        #[allow(dead_code)]
        host: String,
        #[allow(dead_code)]
        port: u16,
    }

    /// The `db` section of the base fixture.
    fn rejecting_builder() -> dynamic_config::Builder<Rejecting> {
        Rejecting::builder("db").file("tests/fixtures/base.json")
    }

    #[test]
    fn a_document_that_does_not_fit_leaves_the_snapshot_alone() {
        rejecting_builder().init().expect("the file is fine");

        let failure = Rejecting::remote_sink()
            .apply(Fetched::new(
                r#"{"db": {"port": "not a number"}}"#,
                Format::Json,
            ))
            .expect_err("the store is pushing something this program cannot use");

        assert_eq!(failure.kind(), dynamic_config::ErrorKind::Type);
        assert_eq!(
            Rejecting::current().port,
            5432,
            "the previous snapshot must still be serving"
        );

        // The bad document stays installed on purpose: it is what the store
        // currently says. Replacing it is a decision for the caller.
        Rejecting::clear_remote();
        assert!(
            rejecting_builder().load().is_ok(),
            "and clearing it recovers"
        );
    }

    #[test]
    fn dropping_the_handle_stops_the_loop() {
        let watch = RemoteWatch::new();
        let watching = watch.watching();

        assert!(watching.keep_going());

        drop(watch);

        assert!(
            !watching.keep_going(),
            "a handle nobody holds must not leave a thread spinning"
        );
    }

    #[test]
    fn stopping_is_visible_without_dropping() {
        let watch = RemoteWatch::new();
        let watching = watch.watching();

        watch.stop();

        assert!(watch.is_stopped());
        assert!(!watching.keep_going());

        // Still bound, so this is not the drop path doing the work.
        drop(watch);
    }

    #[test]
    fn a_detached_handle_watches_for_the_rest_of_the_process() {
        let watch = RemoteWatch::new();
        let watching = watch.watching();

        watch.detach();

        assert!(
            watching.keep_going(),
            "detaching means the loop outlives the handle, not that it stops"
        );
    }

    #[test]
    fn forever_is_a_token_with_no_handle_at_all() {
        assert!(Watching::forever().keep_going());
    }

    #[test]
    fn a_real_loop_runs_until_it_is_told_not_to() {
        let watch = RemoteWatch::new();
        let watching = watch.watching();
        let rounds = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&rounds);

        let thread = std::thread::spawn(move || {
            while watching.keep_going() {
                counted.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        watch.stop();

        thread.join().expect("the loop should end, not hang");

        assert!(
            rounds.load(Ordering::SeqCst) > 1,
            "the loop should have run"
        );
    }
}

/// A container whose configuration comes from a store and the environment has
/// no config file to name, and a builder with no `.file(..)` is how it says so
/// — as opposed to forgetting, which is still an error.
mod fileless {
    use dynamic_config::{dynamic_config, Error, Fetched, Format, RemoteSource};
    use serde::Deserialize;

    #[dynamic_config]
    #[derive(Debug, Deserialize)]
    struct NoFiles {
        #[allow(dead_code)]
        host: String,
        #[allow(dead_code)]
        port: u16,
    }

    /// No files at all: the store and `DCFILELESS_DB_*` are the sources.
    fn no_files_builder() -> dynamic_config::Builder<NoFiles> {
        NoFiles::builder("db").env("DCFILELESS_")
    }

    /// A store with one fixed document.
    struct Store;

    impl RemoteSource for Store {
        fn fetch(&self) -> Result<Fetched, Error> {
            Ok(Fetched::new(
                r#"{"db": {"host": "from-the-store", "port": 5432}}"#,
                Format::Json,
            ))
        }

        fn describe(&self) -> String {
            "a fileless store".to_owned()
        }
    }

    #[test]
    fn a_store_and_the_environment_are_a_whole_configuration() {
        // Nothing on disk, and nothing in the environment yet.
        assert!(
            no_files_builder().load().is_err(),
            "with no source at all there is nothing to load"
        );

        NoFiles::set_remote(Store);
        NoFiles::refresh_remote().expect("the store answers");

        no_files_builder()
            .init()
            .expect("the store alone is enough");
        assert_eq!(NoFiles::current().host, "from-the-store");

        std::env::set_var("DCFILELESS_DB_HOST", "from-the-machine");

        let initialized = no_files_builder().init();
        std::env::remove_var("DCFILELESS_DB_HOST");

        initialized.expect("and the environment still layers over it");
        assert_eq!(NoFiles::current().host, "from-the-machine");
    }
}

/// The push-side fence: a sink taken for one source refuses to deliver
/// after that source is replaced — by construction, not by a request in
/// the documentation to please stop the loop first.
#[test]
fn a_stale_sink_cannot_overwrite_the_store_that_followed_it() {
    use serde::Deserialize;

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Fenced {
        greeting: String,
    }

    struct StoreA;
    impl dynamic_config::RemoteSource for StoreA {
        fn describe(&self) -> String {
            "store A".to_owned()
        }

        fn fetch(&self) -> Result<dynamic_config::Fetched, dynamic_config::Error> {
            Ok(dynamic_config::Fetched::new(
                r#"{"svc": {"greeting": "from A"}}"#,
                dynamic_config::Format::Json,
            ))
        }
    }

    struct StoreB;
    impl dynamic_config::RemoteSource for StoreB {
        fn describe(&self) -> String {
            "store B".to_owned()
        }

        fn fetch(&self) -> Result<dynamic_config::Fetched, dynamic_config::Error> {
            Ok(dynamic_config::Fetched::new(
                r#"{"svc": {"greeting": "from B"}}"#,
                dynamic_config::Format::Json,
            ))
        }
    }

    Fenced::set_remote(StoreA);
    Fenced::refresh_remote().expect("store A answers");
    Fenced::builder("svc")
        .init()
        .expect("the document is complete");

    // The wiring moment: this sink belongs to store A.
    let stale = Fenced::remote_sink();

    // The operator switches stores; A's loop has not noticed yet.
    Fenced::set_remote(StoreB);
    Fenced::refresh_remote().expect("store B answers");
    let fresh = Fenced::remote_sink();
    fresh
        .apply(dynamic_config::Fetched::new(
            r#"{"svc": {"greeting": "B pushed"}}"#,
            dynamic_config::Format::Json,
        ))
        .expect("the current store's sink delivers");

    // A's last in-flight delivery arrives late — and bounces.
    let error = stale
        .apply(dynamic_config::Fetched::new(
            r#"{"svc": {"greeting": "A pushed after replacement"}}"#,
            dynamic_config::Format::Json,
        ))
        .expect_err("a replaced source's sink must refuse");

    assert!(error.to_string().contains("replaced"), "{error}");
    assert_eq!(
        Fenced::current().greeting,
        "B pushed",
        "the stale push must not have landed"
    );
}

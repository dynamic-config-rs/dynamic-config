//! What a reload reports about itself, and what a scrape may read.
//!
//! Two surfaces with one rule between them: an event field may name a key
//! path, and a metric label may not name anything derived from the
//! document at all. The *value* half of that rule is asserted in
//! `tests/security.rs`, where every other promise of the same shape lives;
//! what is here is the shape — the names, the labels, and the bound on how
//! many series they can become.

#![cfg(all(feature = "telemetry", feature = "json"))]

#[cfg(feature = "tracing")]
mod capture;

use dynamic_config::telemetry::{Exposition, METRIC_NAMES};
use dynamic_config::{ConfigCell, Error, Format, LoadSpec, ReloadReason, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Db {
    #[allow(dead_code)]
    port: u16,
}

/// A real load failure, so the recorded [`FailureStatus`] carries the kind
/// and the key path a real one would — `Error`'s constructors are
/// deliberately not public, and a hand-built error would be testing the
/// test.
fn type_error() -> Error {
    let text = r#"{"db": {"port": "not-a-number"}}"#;
    let sources = [Source::inline(text, Format::Json)];

    dynamic_config::load::<Db>(&LoadSpec::new("db", &sources))
        .expect_err("`port` is a string and the field is a u16")
}

/// A cell installed into `installs` times, with `failures` refusals since.
fn cell(installs: u16, failures: u16) -> ConfigCell<u16> {
    let cell = ConfigCell::new();

    for generation in 0..installs {
        cell.store_with(
            generation,
            ReloadReason::FileChanged("/etc/app/config.toml".into()),
        );
    }

    for _ in 0..failures {
        cell.record_failure(&type_error());
    }

    cell
}

#[test]
fn every_family_is_named_and_typed() {
    let cell = cell(1, 1);
    let mut exposition = Exposition::new();
    exposition.add("db", &cell.status());

    let rendered = exposition.render();

    for name in METRIC_NAMES {
        assert!(
            rendered.contains(&format!("# HELP {name} ")),
            "`{name}` is missing its help: {rendered}"
        );
        assert!(
            rendered.contains(&format!("# TYPE {name} ")),
            "`{name}` is missing its type: {rendered}"
        );
    }

    assert!(rendered.contains(r#"dynamic_config_installs_total{config="db"} 1"#));
    assert!(rendered.contains(r#"dynamic_config_consecutive_failures{config="db"} 1"#));
    assert!(rendered
        .contains(r#"dynamic_config_last_reload_info{config="db",reason="file-changed"} 1"#));
    assert!(rendered.contains(r#"dynamic_config_last_failure_info{config="db",kind="type"} 1"#));
}

/// The reason label is the *category*. `FileChanged` owns a path, a path is
/// unbounded, and a metric dimension that grows with a deployment's
/// filesystem is the failure this whole design is arranged around.
#[test]
fn no_label_carries_a_path_or_a_file_name() {
    let cell = cell(1, 1);

    let mut exposition = Exposition::new();
    exposition.add("db", &cell.status());

    let rendered = exposition.render();

    assert!(
        !rendered.contains("/etc/app/config.toml"),
        "a file name reached a label: {rendered}"
    );
    assert!(
        !rendered.contains("port"),
        "a key path reached a label: {rendered}"
    );
    assert!(
        rendered.contains(r#"kind="type""#),
        "the category is what survives: {rendered}"
    );
}

/// The cardinality claim, as a number: six families, one configuration, and
/// a hundred reloads that must not add a series between them.
#[test]
fn a_hundred_reloads_create_no_new_series() {
    let cell = cell(1, 1);

    let samples = |cell: &ConfigCell<u16>| {
        let mut exposition = Exposition::new();
        exposition.add("db", &cell.status());

        exposition
            .render()
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| {
                line.split(' ')
                    .next()
                    .expect("every sample line has a series")
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };

    let before = samples(&cell);
    assert_eq!(
        before.len(),
        6,
        "six families, one configuration: {before:?}"
    );

    for _ in 0..100 {
        cell.store_with(7, ReloadReason::Manual);
    }

    let after = samples(&cell);
    assert_eq!(
        after.len(),
        6,
        "a reload is a new sample, never a new series: {after:?}"
    );
    assert!(
        after
            .iter()
            .any(|series| series.contains(r#"reason="manual""#)),
        "the reason moved, and it moved in place: {after:?}"
    );
}

/// Absent rather than zero: a configuration that has never been installed
/// has no staleness, and `0` would read as "installed just now".
#[test]
fn a_configuration_with_no_history_reports_only_what_is_true() {
    let cell = ConfigCell::<u16>::new();
    let mut exposition = Exposition::new();
    exposition.add("db", &cell.status());

    let rendered = exposition.render();

    assert!(rendered.contains(r#"dynamic_config_installs_total{config="db"} 0"#));
    assert!(rendered.contains(r#"dynamic_config_consecutive_failures{config="db"} 0"#));
    assert!(
        !rendered.contains("dynamic_config_last_success_seconds"),
        "nothing has been installed, so there is no staleness: {rendered}"
    );
    assert!(
        !rendered.contains("dynamic_config_last_failure"),
        "and nothing has failed: {rendered}"
    );
}

/// Several configurations in one exposition, with a family's samples
/// written together — which the text format requires, and a parser that
/// meets `# TYPE` twice for one name rejects the scrape.
#[test]
fn one_family_is_written_once_however_many_configurations_it_covers() {
    let first = cell(1, 0);
    let second = cell(2, 0);

    let mut exposition = Exposition::new();
    exposition.add("db", &first.status());
    exposition.add("cache", &second.status());

    let rendered = exposition.render();

    assert_eq!(
        rendered
            .matches("# TYPE dynamic_config_installs_total")
            .count(),
        1,
        "one header for the family: {rendered}"
    );
    assert!(rendered.contains(r#"dynamic_config_installs_total{config="db"} 1"#));
    assert!(rendered.contains(r#"dynamic_config_installs_total{config="cache"} 2"#));
}

/// Labels of the caller's own, for a process whose configurations need two
/// dimensions — a config server's application and profile.
#[test]
fn a_caller_supplies_its_own_labels() {
    let cell = cell(1, 0);
    let mut exposition = Exposition::new();
    exposition.add_with(
        &[("application", "billing"), ("profile", "prod")],
        &cell.status(),
    );

    assert!(exposition
        .render()
        .contains(r#"dynamic_config_installs_total{application="billing",profile="prod"} 1"#));
}

/// A label value cannot end its own line. What a caller passes is theirs —
/// a section name, a service name — and a format where a quote or a newline
/// forges a second sample is a format that trusts its input.
#[test]
fn a_label_cannot_forge_a_sample_line() {
    let cell = cell(1, 0);
    let mut exposition = Exposition::new();
    exposition.add_with(
        &[("pod-name", "a\" } 999\ndynamic_config_installs_total 42")],
        &cell.status(),
    );

    let rendered = exposition.render();

    assert!(
        rendered.contains(r#"pod_name="a\" } 999\ndynamic_config_installs_total 42""#),
        "the value escaped, and the label name coerced: {rendered}"
    );
    assert_eq!(
        rendered
            .lines()
            .filter(|line| line.starts_with("dynamic_config_installs_total"))
            .count(),
        1,
        "one sample, not two: {rendered}"
    );
}

/// The fetch half: what a remote source reports about itself, and what it
/// still may not say.
///
/// One `Remote` per test — it is a slot with recorded state, so two tests
/// sharing one would be the shared-state mistake `AGENTS.md` names.
mod remote {
    use std::sync::atomic::{AtomicBool, Ordering};

    use dynamic_config::telemetry::{Exposition, REMOTE_METRIC_NAMES};
    use dynamic_config::{Error, Fetched, Format, Remote, RemoteSource};

    /// A store whose description is a URL with a credential in it — which is
    /// what a real one's is. Nothing rendered may contain it.
    pub(super) const URL: &str = "https://consul:hunter2-do-not-print-me@store.internal:8500";

    pub(super) struct Store {
        answers: bool,
    }

    impl Store {
        pub(super) fn answering() -> Self {
            Self { answers: true }
        }

        pub(super) fn silent() -> Self {
            Self { answers: false }
        }
    }

    impl RemoteSource for Store {
        fn fetch(&self) -> Result<Fetched, Error> {
            if self.answers {
                Ok(Fetched::new(r#"{"db": {"port": 5432}}"#, Format::Json))
            } else {
                Err(Error::remote("the store is unreachable"))
            }
        }

        fn describe(&self) -> String {
            URL.to_owned()
        }
    }

    /// A store that answers once and then goes away, so one `Remote` can be
    /// watched crossing from up to down.
    struct Fading(AtomicBool);

    impl RemoteSource for Fading {
        fn fetch(&self) -> Result<Fetched, Error> {
            if self.0.swap(true, Ordering::SeqCst) {
                return Store::silent().fetch();
            }

            Store::answering().fetch()
        }

        fn describe(&self) -> String {
            URL.to_owned()
        }
    }

    fn rendered(remote: &Remote) -> String {
        let mut exposition = Exposition::new();
        exposition.add_remote("store", &remote.status());

        exposition.render()
    }

    /// The sample lines, as series names — what a cardinality claim counts.
    fn series(rendered: &str) -> Vec<String> {
        rendered
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| {
                line.split(' ')
                    .next()
                    .expect("every sample line has a series")
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn a_fetch_reports_that_the_store_answered_and_how_long_it_took() {
        let remote = Remote::new();
        remote.set(Store::answering());
        remote.refresh().expect("the store answers");

        let rendered = rendered(&remote);

        for name in REMOTE_METRIC_NAMES {
            // `last_failure_info` has not happened; everything else has.
            if name.ends_with("last_failure_info") {
                continue;
            }

            assert!(
                rendered.contains(&format!("# TYPE {name} ")),
                "`{name}` is missing: {rendered}"
            );
        }

        assert!(rendered.contains(r#"dynamic_config_remote_up{config="store"} 1"#));
        assert!(rendered.contains(r#"dynamic_config_remote_fetches_total{config="store"} 1"#));
        assert!(
            rendered.contains(r#"dynamic_config_remote_consecutive_failures{config="store"} 0"#)
        );
        assert!(
            rendered
                .contains(r#"dynamic_config_remote_last_fetch_duration_seconds{config="store"}"#),
            "a pull is timed: {rendered}"
        );
    }

    /// The claim item 24 parked, as a number: a store stops answering,
    /// `remote_up` falls to zero, and the series count does not move.
    #[test]
    fn a_store_that_stops_answering_falls_to_zero_without_adding_a_series() {
        let remote = Remote::new();
        remote.set(Fading(AtomicBool::new(false)));
        remote.refresh().expect("the first fetch answers");

        let before = series(&rendered(&remote));
        assert_eq!(
            before.len(),
            5,
            "five families before anything has failed: {before:?}"
        );

        for _ in 0..50 {
            remote.refresh().expect_err("the store has gone away");
        }

        let after = rendered(&remote);
        let names = series(&after);

        assert!(
            after.contains(r#"dynamic_config_remote_up{config="store"} 0"#),
            "the store is down and the gauge says so: {after}"
        );
        assert!(
            after.contains(r#"dynamic_config_remote_consecutive_failures{config="store"} 50"#),
            "{after}"
        );
        assert!(
            after.contains(
                r#"dynamic_config_remote_last_failure_info{config="store",kind="remote"} 1"#
            ),
            "{after}"
        );
        assert_eq!(
            names.len(),
            6,
            "fifty failures add one family — the failure category — and no more: {names:?}"
        );

        // And the document the store did answer with is still in the slot:
        // reporting the outage is not the same as forgetting the value.
        assert!(remote.document().is_some());
    }

    /// Absent rather than zero. A source that has been installed and never
    /// asked is not *down*, and a `0` at startup is a page nobody should be
    /// woken by.
    #[test]
    fn a_store_nobody_has_asked_is_not_reported_as_down() {
        let remote = Remote::new();
        remote.set(Store::answering());

        let rendered = rendered(&remote);

        assert!(
            !rendered.contains("dynamic_config_remote_up"),
            "nothing has been asked of it yet: {rendered}"
        );
        assert!(rendered.contains(r#"dynamic_config_remote_fetches_total{config="store"} 0"#));
        assert!(
            !rendered.contains("dynamic_config_remote_last_fetch_seconds"),
            "and there is no staleness either: {rendered}"
        );
    }

    /// Replacing the source drops the recorded history with the document: a
    /// `1` describing a store nobody is talking to any more is worse than no
    /// sample at all.
    #[test]
    fn replacing_the_source_drops_what_was_recorded_about_the_old_one() {
        let remote = Remote::new();
        remote.set(Store::silent());
        remote.refresh().expect_err("the store is unreachable");

        assert!(rendered(&remote).contains(r#"dynamic_config_remote_up{config="store"} 0"#));

        remote.set(Store::answering());

        assert!(
            !rendered(&remote).contains("dynamic_config_remote_up"),
            "the new source has not been asked anything yet"
        );
    }

    /// **No store URL reaches a label**, and the referee for the whole rule
    /// is `tests/security.rs`; this is the cardinality half of it. The name
    /// on a series is the caller's, because the only string a source has is
    /// a URL with a password in it.
    #[test]
    fn the_series_are_named_by_the_caller_and_never_by_the_store() {
        let remote = Remote::new();
        remote.set(Store::silent());
        remote.refresh().expect_err("the store is unreachable");

        let rendered = rendered(&remote);

        assert!(!rendered.contains("store.internal"), "{rendered}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert_eq!(
            remote.describe().as_deref(),
            Some(URL),
            "the description is still available to a caller that asks for it"
        );
    }
}

/// The push half: a document a watch loop delivered is a fetch somebody else
/// performed, and is counted as one — with **no** duration, because nobody
/// here timed it.
///
/// Its own `static`, because a `RemoteSink` needs one; no other test in this
/// file touches it.
#[test]
fn a_pushed_document_counts_as_a_fetch_and_claims_no_duration() {
    use dynamic_config::{Fetched, Remote, RemoteSink};

    static PUSHED: Remote = Remote::new();

    fn reloaded() -> Result<(), Error> {
        Ok(())
    }

    PUSHED.set(remote::Store::answering());

    // A pull first, so there *is* a duration to lose.
    PUSHED.refresh().expect("the store answers");

    let sink = RemoteSink::new(&PUSHED, reloaded, "pushed");
    sink.apply(Fetched::new(r#"{"db": {"port": 5433}}"#, Format::Json))
        .expect("the source has not moved");

    let mut exposition = Exposition::new();
    exposition.add_remote("pushed", &PUSHED.status());
    let rendered = exposition.render();

    assert!(
        rendered.contains(r#"dynamic_config_remote_fetches_total{config="pushed"} 2"#),
        "one pulled, one pushed: {rendered}"
    );
    assert!(
        rendered.contains(r#"dynamic_config_remote_up{config="pushed"} 1"#),
        "{rendered}"
    );
    assert!(
        !rendered.contains("dynamic_config_remote_last_fetch_duration_seconds"),
        "a pushed document was timed by the store crate that fetched it, and \
         reporting the previous pull's duration beside it would be a number \
         about the wrong fetch: {rendered}"
    );
}

/// The records a reload emits, read back through the suite's capturing
/// subscriber.
///
/// One configuration type per test, and each test reads only the records
/// naming *its* type: the subscriber is the binary's, so the tests would
/// otherwise read each other's reloads.
#[cfg(feature = "tracing")]
mod events {
    use dynamic_config::{ConfigCell, ReloadReason};

    struct Installed(#[allow(dead_code)] u16);
    struct Refused;
    struct Quiet(#[allow(dead_code)] u16);

    #[test]
    fn an_install_is_a_span_carrying_the_reason_and_the_generation() {
        let captured = super::capture::global();

        let cell = ConfigCell::new();
        cell.store_with(Installed(1), ReloadReason::Initial);
        cell.store_with(
            Installed(2),
            ReloadReason::FileChanged("/etc/app/config.toml".into()),
        );

        let lines = captured.about("events::Installed");

        assert!(
            lines.contains("span dynamic_config.reload"),
            "no reload span: {lines}"
        );
        assert!(lines.contains(r#"reason="initial""#), "{lines}");
        assert!(lines.contains(r#"reason="file-changed""#), "{lines}");
        assert!(lines.contains("generation=1"), "{lines}");
        assert!(lines.contains("generation=2"), "{lines}");
        assert!(lines.contains(r#"outcome="installed""#), "{lines}");
        assert!(
            !lines.contains("/etc/app/config.toml"),
            "the reason is the category; the path is unbounded: {lines}"
        );
    }

    #[test]
    fn a_refusal_is_an_event_carrying_the_kind_and_the_key_path() {
        let captured = super::capture::global();

        ConfigCell::<Refused>::new().record_failure(&super::type_error());

        let lines = captured.about("events::Refused");

        assert!(lines.contains("event WARN"), "{lines}");
        assert!(lines.contains(r#"outcome="rejected""#), "{lines}");
        assert!(lines.contains(r#"error.kind="type""#), "{lines}");
        assert!(
            lines.contains(r#"error.path="port""#),
            "the key path is the actionable half of a failure: {lines}"
        );
    }

    /// A fetch is a span with an event inside it, and the event carries the
    /// outcome and — on a failure — the category, and nothing else.
    ///
    /// Both outcomes in one test on purpose: the fetch records carry no
    /// configuration type to filter on (a `Remote` has no name that is not
    /// its store's URL), so this is the only test in the binary that fetches
    /// and the lines it reads are therefore its own.
    #[test]
    fn a_fetch_is_a_span_and_an_event_carrying_the_outcome_and_the_kind() {
        let captured = super::capture::global();

        let remote = dynamic_config::Remote::new();

        remote.set(super::remote::Store::answering());
        remote.refresh().expect("the store answers");

        remote.set(super::remote::Store::silent());
        remote.refresh().expect_err("the store is unreachable");

        let lines = captured.about("dynamic_config.fetch");
        assert!(
            lines.lines().count() >= 2,
            "a span per fetch: {}",
            captured.joined()
        );

        let events = captured.about("a remote store answered");
        assert!(events.contains(r#"outcome="fetched""#), "{events}");
        assert!(events.contains("duration_ms="), "{events}");

        let failures = captured.about("returned nothing");
        assert!(failures.contains("event WARN"), "{failures}");
        assert!(failures.contains(r#"outcome="failed""#), "{failures}");
        assert!(failures.contains(r#"error.kind="remote""#), "{failures}");

        // The store's URL is the only name a source has, and it carries a
        // credential. Not in a span, not in an event.
        assert!(
            !captured.joined().contains(super::remote::URL),
            "a store URL reached a record: {}",
            captured.joined()
        );
    }

    /// The read path emits nothing, and must not: a span per read is a span
    /// per request in every service that uses this crate.
    #[test]
    fn reading_a_snapshot_emits_nothing() {
        let captured = super::capture::global();
        let cell = ConfigCell::new();

        cell.store(Quiet(1));
        let after_the_install = captured.count("events::Quiet");

        for _ in 0..100 {
            let _ = cell.load();
        }

        assert_eq!(
            captured.count("events::Quiet"),
            after_the_install,
            "a hundred reads added a record: {}",
            captured.about("events::Quiet")
        );
    }
}

/// A watch loop that is *failing* delivers nothing, so `apply` never runs and
/// the store would look healthy on the strength of a delivery an hour old.
/// `failed` is the door for that, and what it moves is narrow on purpose: the
/// streak and the last failure go, while `last_fetch` keeps ageing — an alert
/// wants "up went to zero *and* the last good read is getting old", and a
/// failure that reset the clock would hide the second half.
#[test]
fn a_failing_watch_loop_reports_without_disturbing_the_last_good_read() {
    use dynamic_config::{Remote, RemoteSink};

    static FAILING: Remote = Remote::new();

    fn reloaded() -> Result<(), Error> {
        Ok(())
    }

    FAILING.set(remote::Store::answering());
    FAILING.refresh().expect("the store answers");

    let sink = RemoteSink::new(&FAILING, reloaded, "failing");
    let before = FAILING.status();

    sink.failed(&Error::remote("the subscription dropped"));
    sink.failed(&Error::remote("and again"));

    let after = FAILING.status();

    assert_eq!(after.consecutive_failures, 2);
    assert_eq!(
        after.reachable(),
        Some(false),
        "two failed attempts and no delivery is a store that is not answering"
    );
    assert_eq!(
        after.fetches, before.fetches,
        "an attempt that returned nothing is not a fetch"
    );
    assert_eq!(
        after.last_fetch, before.last_fetch,
        "the last *good* read has to keep ageing, or the staleness alert \
         resets every time the store fails"
    );

    let mut exposition = Exposition::new();
    exposition.add_remote("failing", &after);
    let rendered = exposition.render();

    assert!(
        rendered.contains(r#"dynamic_config_remote_up{config="failing"} 0"#),
        "{rendered}"
    );
    assert!(
        rendered.contains("dynamic_config_remote_last_fetch_seconds"),
        "the staleness series must still be there: {rendered}"
    );
    assert!(
        !rendered.contains("subscription"),
        "a failure's text never becomes a label: {rendered}"
    );

    // A delivery clears the streak, exactly as a successful pull does.
    sink.apply(dynamic_config::Fetched::new(
        r#"{"db": {"port": 5433}}"#,
        Format::Json,
    ))
    .expect("the source has not moved");

    assert_eq!(FAILING.status().consecutive_failures, 0);
}

/// A loop still winding down after its source was replaced must not charge
/// its failures to the replacement — the same fence `apply` has, for the same
/// reason, and silent because a loop must never have to handle a failure to
/// report a failure.
#[test]
fn a_stale_loops_failure_is_dropped_rather_than_charged_to_the_replacement() {
    use dynamic_config::{Remote, RemoteSink};

    static REPLACED: Remote = Remote::new();

    fn reloaded() -> Result<(), Error> {
        Ok(())
    }

    REPLACED.set(remote::Store::answering());

    let stale = RemoteSink::new(&REPLACED, reloaded, "replaced");

    // The source is replaced; the old loop has not noticed yet.
    REPLACED.set(remote::Store::answering());

    stale.failed(&Error::remote("the old subscription dropped"));

    assert_eq!(
        REPLACED.status().consecutive_failures,
        0,
        "the replacement has not failed at anything"
    );
}

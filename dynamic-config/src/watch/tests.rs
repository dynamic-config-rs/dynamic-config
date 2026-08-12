use std::any::TypeId;
use std::path::PathBuf;
use std::time::Duration;

use notify::event::{CreateKind, ModifyKind};
use notify::{Event, EventKind};

use crate::source::LoadSpec;

use super::handle::STARTED;
use super::relevance::is_relevant;
use super::{spawn, Watched};

/// A spec that names one file explicitly and searches nowhere.
fn explicit_spec() -> LoadSpec<'static> {
    static SOURCES: &[crate::Source<'static>] =
        &[crate::Source::file("config.toml", crate::Format::Toml)];

    LoadSpec::new("app", SOURCES)
}

fn event(kind: EventKind, path: &str) -> Event {
    Event {
        kind,
        paths: vec![PathBuf::from(path)],
        attrs: Default::default(),
    }
}

#[test]
fn an_absolute_event_path_matches_a_relative_configured_path() {
    let probe = event(EventKind::Modify(ModifyKind::Any), "/srv/app/config.toml");

    assert!(is_relevant(&probe, &Watched::from_spec(&explicit_spec())));
}

#[test]
fn a_discovered_name_matches_even_though_no_file_was_listed() {
    let paths: &'static [&'static str] = &["/srv/app"];
    let spec = LoadSpec::new("db", &[]).with_search("config", paths);
    let watched = Watched::from_spec(&spec);

    let probe = event(EventKind::Create(CreateKind::File), "/srv/app/config.toml");
    assert!(is_relevant(&probe, &watched));

    let probe = event(EventKind::Create(CreateKind::File), "/srv/app/other.toml");
    assert!(!is_relevant(&probe, &watched));
}

#[test]
fn an_unrelated_file_in_the_same_directory_is_ignored() {
    let probe = event(EventKind::Modify(ModifyKind::Any), "/srv/app/notes.txt");

    assert!(!is_relevant(&probe, &Watched::from_spec(&explicit_spec())));
}

#[test]
fn access_events_do_not_trigger_a_reload() {
    let probe = event(
        EventKind::Access(notify::event::AccessKind::Read),
        "/srv/app/config.toml",
    );

    assert!(!is_relevant(&probe, &Watched::from_spec(&explicit_spec())));
}

#[test]
fn a_duplicate_spawn_is_an_error_and_frees_nothing() {
    struct DuplicateMarker;

    let spec = explicit_spec();
    let key = super::WatchKey::Type(TypeId::of::<DuplicateMarker>());

    let first = spawn(
        key,
        "DuplicateTest",
        Watched::from_spec(&spec),
        Duration::from_millis(10),
        || Ok(None),
    )
    .expect("the first spawn should start a watcher");

    // The second is refused loudly — the old inert-handle return was a
    // success nobody could distinguish from the real thing.
    let spec = explicit_spec();
    let error = spawn(
        key,
        "DuplicateTest",
        Watched::from_spec(&spec),
        Duration::from_millis(10),
        || Ok(None),
    )
    .expect_err("a second watcher for the same type must be refused");

    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(error.to_string().contains("DuplicateTest"), "{error}");
    assert!(
        STARTED.lock().unwrap().contains_key(&key),
        "the refusal must not free the first watcher's registration"
    );

    drop(first);
    assert!(
        !STARTED.lock().unwrap().contains_key(&key),
        "dropping the real handle frees the registration"
    );

    // And a fresh spawn works again — the refusal did not wedge the slot.
    let spec = explicit_spec();
    let again = spawn(
        key,
        "DuplicateTest",
        Watched::from_spec(&spec),
        Duration::from_millis(10),
        || Ok(None),
    )
    .expect("after the drop, watching can restart");
    drop(again);
}

/// A failed spawn must free its name, or every retry afterwards returns a
/// success handle that owns nothing and watches nothing — silently, which
/// is the exact path a program hits when its config directory does not
/// exist yet at startup.
#[test]
fn a_failed_spawn_frees_its_registration_for_a_retry() {
    struct FailedSpawnMarker;

    let key = super::WatchKey::Type(TypeId::of::<FailedSpawnMarker>());

    static BAD: [crate::Source<'static>; 1] = [crate::Source::file(
        "/nonexistent-dynamic-config-test-dir/config.toml",
        crate::Format::Toml,
    )];
    let bad = LoadSpec::new("db", &BAD);

    let _ = spawn(
        key,
        "FailedSpawnTest",
        Watched::from_spec(&bad),
        Duration::from_millis(10),
        || Ok(None),
    )
    .expect_err("no directory to watch means the spawn fails");

    assert!(
        !STARTED.lock().unwrap().contains_key(&key),
        "a failed spawn must not keep its registration"
    );

    // The retry gets a real watcher, not an inert one.
    let handle = spawn(
        key,
        "FailedSpawnTest",
        Watched::from_spec(&explicit_spec()),
        Duration::from_millis(10),
        || Ok(None),
    )
    .expect("the retry should start a watcher");

    drop(handle);
    assert!(
        !STARTED.lock().unwrap().contains_key(&key),
        "and dropping it frees the registration"
    );
}

#[test]
fn creation_and_removal_both_count_as_changes() {
    for kind in [
        EventKind::Create(CreateKind::File),
        EventKind::Remove(notify::event::RemoveKind::File),
    ] {
        let probe = event(kind, "config.toml");

        assert!(
            is_relevant(&probe, &Watched::from_spec(&explicit_spec())),
            "{kind:?}"
        );
    }
}

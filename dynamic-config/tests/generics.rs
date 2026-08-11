//! Generic configuration types.
//!
//! `Config<Postgres>` and `Config<Mysql>` are different types, so they get
//! different snapshots — which is the whole point, and the reason a `static`
//! cannot hold them.

#![cfg(feature = "json")]

use std::marker::PhantomData;

use dynamic_config::dynamic_config;
use serde::Deserialize;

trait Driver {
    const NAME: &'static str;
}

struct Postgres;
struct Mysql;

impl Driver for Postgres {
    const NAME: &'static str = "postgres";
}

impl Driver for Mysql {
    const NAME: &'static str = "mysql";
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Db<D: Driver> {
    host: String,
    port: u16,
    // `fn() -> D` rather than `D`, so the marker stays `Send + Sync` whatever
    // `D` is — the usual shape for a type-level tag.
    #[serde(skip)]
    driver: PhantomData<fn() -> D>,
}

impl<D: Driver> Db<D> {
    fn replacement(host: &str) -> Self {
        Self {
            host: host.to_owned(),
            port: 1,
            driver: PhantomData,
        }
    }
}

/// The `db` section of the base fixture, for whichever instantiation asks.
fn db_sources<D: Driver + 'static>() -> dynamic_config::Builder<Db<D>> {
    Db::<D>::builder("db").file("tests/fixtures/base.json")
}

#[test]
fn every_instantiation_loads_independently() {
    let postgres = db_sources::<Postgres>()
        .load()
        .expect("the fixture is complete");
    let mysql = db_sources::<Mysql>()
        .load()
        .expect("the fixture is complete");

    assert_eq!(postgres.host, "localhost");
    assert_eq!(mysql.port, 5432);
    assert_eq!(Postgres::NAME, "postgres");
}

#[test]
fn snapshots_do_not_leak_between_instantiations() {
    db_sources::<Postgres>()
        .init()
        .expect("the fixture is complete");
    db_sources::<Mysql>()
        .init()
        .expect("the fixture is complete");

    Db::<Postgres>::replace(Db::<Postgres>::replacement("only-postgres"));

    assert_eq!(Db::<Postgres>::current().host, "only-postgres");
    assert_eq!(
        Db::<Mysql>::current().host,
        "localhost",
        "one instantiation must not write over another's snapshot"
    );
}

#[test]
fn the_runtime_layers_are_per_instantiation_too() {
    Db::<Postgres>::set_override("port", 9999u16).unwrap();

    assert_eq!(db_sources::<Postgres>().load().unwrap().port, 9999);
    assert_eq!(
        db_sources::<Mysql>().load().unwrap().port,
        5432,
        "an override on one instantiation must not reach another"
    );

    Db::<Postgres>::clear_overrides();
}

/// A parameter named like the ones the macro introduces for its own methods.
///
/// `quote!` is not hygienic for generic parameters, so `set_override<T>` in the
/// generated code would collide with a caller's `T`. This is the regression
/// test for that.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Awkward<T, F> {
    host: String,
    #[allow(dead_code)]
    port: u16,
    #[serde(skip)]
    marker: PhantomData<fn() -> (T, F)>,
}

#[test]
fn a_parameter_named_t_does_not_collide_with_the_generated_methods() {
    Awkward::<u8, u16>::set_override("host", "overridden").unwrap();

    assert_eq!(
        Awkward::<u8, u16>::builder("db")
            .file("tests/fixtures/base.json")
            .load()
            .unwrap()
            .host,
        "overridden"
    );

    Awkward::<u8, u16>::on_reload(|_, _| {});
    Awkward::<u8, u16>::clear_overrides();
}

/// Const generics key the registry just as well as types do.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Sized_<const N: usize> {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn const_parameters_get_their_own_slots() {
    Sized_::<1>::builder("db")
        .file("tests/fixtures/base.json")
        .init()
        .expect("the fixture is complete");
    Sized_::<2>::builder("db")
        .file("tests/fixtures/base.json")
        .init()
        .expect("the fixture is complete");

    Sized_::<1>::replace(Sized_::<1> {
        host: "one".to_owned(),
        port: 1,
    });

    assert_eq!(Sized_::<1>::current().host, "one");
    assert_eq!(Sized_::<2>::current().host, "localhost");
}

/// A generic type reaches its remote slot through the `TypeId` registry rather
/// than through a `static`, so the watch sink has to work there too — and each
/// instantiation must keep its own document.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Watched<T> {
    host: String,
    #[allow(dead_code)]
    port: u16,
    #[serde(skip)]
    #[allow(dead_code)]
    marker: std::marker::PhantomData<T>,
}

struct Primary;
struct Replica;

#[test]
fn a_pushed_document_lands_in_one_instantiation_only() {
    use dynamic_config::{Fetched, Format};

    Watched::<Primary>::builder("db")
        .file("tests/fixtures/base.json")
        .init()
        .expect("the fixture is complete");
    Watched::<Replica>::builder("db")
        .file("tests/fixtures/base.json")
        .init()
        .expect("the fixture is complete");

    Watched::<Primary>::apply_remote(Fetched::new(
        r#"{"db": {"host": "primary.internal"}}"#,
        Format::Json,
    ))
    .expect("the document merges over the file");

    assert_eq!(Watched::<Primary>::current().host, "primary.internal");
    assert_eq!(
        Watched::<Replica>::current().host,
        "localhost",
        "one instantiation's remote store must not reach another's"
    );
}

// Its own type and its own scratch files: watch tests share nothing.
#[cfg(feature = "watch")]
#[dynamic_config]
#[derive(Debug, serde::Deserialize)]
struct PerInstance<T> {
    #[allow(dead_code)]
    value: u32,
    #[serde(skip)]
    #[allow(dead_code)]
    marker: std::marker::PhantomData<T>,
}

/// A poll watcher over this test's scratch file, for one instantiation.
#[cfg(feature = "watch")]
fn per_instance_watch<T: Send + Sync + 'static>(
) -> std::io::Result<dynamic_config::watch::WatchHandle> {
    use std::time::Duration;

    PerInstance::<T>::builder("db")
        .file("tests/scratch/generic-watch.json")
        .watch_with(
            Duration::from_millis(50),
            dynamic_config::watch::WatchMode::Poll {
                interval: Duration::from_millis(100),
            },
        )
}

#[cfg(feature = "watch")]
struct Alpha;
#[cfg(feature = "watch")]
struct Beta;

/// The bug this pins: watchers used to be keyed by the bare type *name*, so
/// `PerInstance<Alpha>` and `PerInstance<Beta>` collided on "PerInstance" —
/// the second `start_watch()` silently watched nothing. Keyed by `TypeId`,
/// each instantiation gets its own watcher, and a true duplicate is a loud
/// error instead of an inert handle.
#[cfg(feature = "watch")]
#[test]
fn each_instantiation_gets_its_own_watcher() {
    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/generic-watch.json",
        r#"{"db": {"value": 1}}"#,
    )
    .unwrap();

    let alpha = per_instance_watch::<Alpha>().expect("the first instantiation watches");
    let beta = per_instance_watch::<Beta>()
        .expect("a different instantiation is a different watcher, not a duplicate");

    // A genuine duplicate is refused loudly.
    let error =
        per_instance_watch::<Alpha>().expect_err("the same instantiation twice is an error");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);

    drop(alpha);
    drop(beta);

    // And once dropped, both restart cleanly.
    per_instance_watch::<Alpha>()
        .expect("after the drop, watching can restart")
        .stop();
}

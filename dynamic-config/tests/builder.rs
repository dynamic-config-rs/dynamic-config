//! The builder: runtime-chosen sources, same semantics as the attribute.

#![cfg(feature = "json")]

use serde::Deserialize;

#[test]
fn a_bare_builder_loads_without_any_macro() {
    #[derive(Debug, Deserialize)]
    struct Db {
        host: String,
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/builder-bare.json",
        r#"{"db": {"host": "localhost", "port": 5432}}"#,
    )
    .unwrap();

    let db: Db = dynamic_config::Builder::new("db")
        .file("tests/scratch/builder-bare.json")
        .load()
        .expect("the source reads cleanly");

    assert_eq!(db.host, "localhost");
    assert_eq!(db.port, 5432);
}

#[test]
fn a_bare_builder_refuses_to_init() {
    #[derive(Debug, Deserialize)]
    struct Db {
        #[allow(dead_code)]
        #[serde(default)]
        port: u16,
    }

    let error = dynamic_config::Builder::<Db>::new("db")
        .init()
        .expect_err("there is no storage to install into");

    assert!(error.to_string().contains("builder()"), "{error}");
}

#[test]
fn the_generated_builder_installs_into_the_same_snapshot() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Runtime {
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/builder-generated.json",
        r#"{"svc": {"port": 7777}}"#,
    )
    .unwrap();

    Runtime::builder("svc")
        .file("tests/scratch/builder-generated.json")
        .init()
        .expect("the source reads cleanly");

    assert_eq!(
        Runtime::current().port,
        7777,
        "current() sees what init installed"
    );
}

#[test]
fn the_builder_speaks_strict_env_too() {
    #[derive(Debug, Deserialize)]
    struct Svc {
        #[allow(dead_code)]
        #[serde(default)]
        mode: String,
    }

    std::env::set_var("BUILDERSTRICT_SVC_MODE", "off");

    let error = dynamic_config::Builder::<Svc>::new("svc")
        .env("BUILDERSTRICT_")
        .strict_env()
        .load()
        .expect_err("the same refusal as the attribute's strict_env");

    assert!(
        error.to_string().contains("BUILDERSTRICT_SVC_MODE"),
        "{error}"
    );

    std::env::remove_var("BUILDERSTRICT_SVC_MODE");
}

#[test]
fn the_builder_explains_like_everything_else() {
    #[derive(Debug, Deserialize)]
    struct Db {
        #[allow(dead_code)]
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/builder-explain.json",
        r#"{"db": {"port": 6543}}"#,
    )
    .unwrap();

    let explanation = dynamic_config::Builder::<Db>::new("db")
        .file("tests/scratch/builder-explain.json")
        .explain("port")
        .expect("the source reads cleanly");

    assert_eq!(explanation.winner().unwrap().layer, "file");
}

#[test]
fn a_bare_builder_refuses_a_redacted_cache() {
    #[derive(Debug, Deserialize)]
    struct Db {
        #[allow(dead_code)]
        #[serde(default)]
        port: u16,
    }

    let error = dynamic_config::Builder::<Db>::new("db")
        .cache(
            "tests/scratch/builder-refused-cache.json",
            dynamic_config::CacheMode::Redacted,
        )
        .init()
        .expect_err("redaction needs the generated builder's secret knowledge");

    assert!(error.to_string().contains("secret"), "{error}");
}

#[test]
fn the_generated_builder_recovers_from_its_cache() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Recovering {
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/builder-recover.json",
        r#"{"svc": {"port": 4444}}"#,
    )
    .unwrap();

    let good = Recovering::builder("svc")
        .file("tests/scratch/builder-recover.json")
        .cache(
            "tests/scratch/builder-recover-cache.json",
            dynamic_config::CacheMode::Full,
        );
    good.init()
        .expect("the source reads cleanly, and the cache is written");

    // The source breaks; the cache carries the last configuration that worked.
    std::fs::write("tests/scratch/builder-recover.json", "{ not json").unwrap();

    good.init().expect("recovery from the cache");
    assert_eq!(Recovering::current().port, 4444);
}

#[cfg(feature = "watch")]
#[test]
fn the_builder_watch_reloads_into_the_same_snapshot() {
    use std::time::{Duration, Instant};

    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct BuilderWatched {
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/builder-watch.json",
        r#"{"svc": {"port": 1000}}"#,
    )
    .unwrap();

    let builder = BuilderWatched::builder("svc").file("tests/scratch/builder-watch.json");
    builder.init().expect("the source reads cleanly");

    let _watch = builder
        .watch(Duration::from_millis(50))
        .expect("the watcher starts");

    // The same registry as start_watch: a second watch on the type is refused.
    let duplicate = builder.watch(Duration::from_millis(50));
    assert_eq!(
        duplicate.expect_err("one watcher per type").kind(),
        std::io::ErrorKind::AlreadyExists
    );

    std::fs::write(
        "tests/scratch/builder-watch.json",
        r#"{"svc": {"port": 2000}}"#,
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);
    while BuilderWatched::current().port != 2000 {
        assert!(Instant::now() < deadline, "the reload never landed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Fingerprint promises to diagnose and never recover — even from a
/// value-bearing cache an earlier deployment left at the same path.
#[test]
fn a_fingerprint_builder_never_boots_from_a_value_cache() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Fingerprinted {
        #[allow(dead_code)]
        port: u16,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/builder-fingerprint.json",
        r#"{"svc": {"port": 5555}}"#,
    )
    .unwrap();

    // An earlier deployment cached values at this path.
    Fingerprinted::builder("svc")
        .file("tests/scratch/builder-fingerprint.json")
        .cache(
            "tests/scratch/builder-fingerprint-cache.json",
            dynamic_config::CacheMode::Full,
        )
        .init()
        .expect("the source reads cleanly; the cache is written");

    // This deployment is configured to diagnose only.
    std::fs::write("tests/scratch/builder-fingerprint.json", "{ not json").unwrap();

    Fingerprinted::builder("svc")
        .file("tests/scratch/builder-fingerprint.json")
        .cache(
            "tests/scratch/builder-fingerprint-cache.json",
            dynamic_config::CacheMode::Fingerprint,
        )
        .init()
        .expect_err("the on-disk values must not override the configured mode");
}

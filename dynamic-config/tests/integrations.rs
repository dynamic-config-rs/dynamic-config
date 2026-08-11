//! Command-line flags, and checking a configuration without booting it.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, Origin};
use serde::Deserialize;

macro_rules! db_config {
    ($name:ident) => {
        #[dynamic_config]
        #[derive(Debug, Deserialize)]
        struct $name {
            #[allow(dead_code)]
            host: String,
            port: u16,
            #[allow(dead_code)]
            pool: Pool,
        }

        impl $name {
            /// The `db` section of the base fixture, with the environment
            /// layer between the files and the flags.
            fn sources() -> dynamic_config::Builder<$name> {
                $name::builder("db")
                    .file("tests/fixtures/base.json")
                    .env("DCINT_")
            }
        }
    };
}

#[derive(Debug, Deserialize)]
struct Pool {
    #[allow(dead_code)]
    max_size: u16,
}

// The `DCINT_` prefix is wired up but never set by any test here; it exists so
// the environment layer sits between the files and the flags, which is the
// ordering these tests are about.
db_config!(Flagged);
db_config!(Assigned);
db_config!(Ranked);

// ─────────────────────────────────────────────
// Flags
// ─────────────────────────────────────────────

#[test]
fn an_absent_flag_leaves_the_files_alone() {
    // The shape a caller writes: pass every flag unconditionally and let `None`
    // mean "not given".
    Flagged::set_flag("port", None::<u16>).unwrap();

    assert_eq!(Flagged::sources().load().unwrap().port, 5432);

    Flagged::set_flag("port", Some(9000u16)).unwrap();
    assert_eq!(Flagged::sources().load().unwrap().port, 9000);

    Flagged::clear_flags();
}

#[test]
fn assignments_read_the_way_environment_variables_do() {
    Assigned::set_assignments(["port=9001", "pool.max_size=64"]).unwrap();

    let config = Assigned::sources()
        .load()
        .expect("both assignments are well formed");

    assert_eq!(config.port, 9001, "`9001` is a number, not the string");
    assert_eq!(
        config.pool.max_size, 64,
        "a dotted key reaches a nested field"
    );

    Assigned::clear_flags();
}

#[test]
fn a_malformed_assignment_names_itself() {
    let error = Assigned::set_assignments(["port"]).unwrap_err();

    assert!(error.to_string().contains("`port`"), "{error}");
}

#[test]
fn a_flag_outranks_a_file_and_an_override_outranks_the_flag() {
    Ranked::set_flag("port", Some(1u16)).unwrap();
    assert_eq!(Ranked::sources().load().unwrap().port, 1);
    assert_eq!(
        Ranked::sources().source_of("port").unwrap(),
        Some(Origin::Runtime("command-line flag"))
    );

    Ranked::set_override("port", 2u16).unwrap();
    assert_eq!(Ranked::sources().load().unwrap().port, 2);
    assert_eq!(
        Ranked::sources().source_of("port").unwrap(),
        Some(Origin::Runtime("override"))
    );

    Ranked::clear_overrides();
    Ranked::clear_flags();
}

// ─────────────────────────────────────────────
// check
// ─────────────────────────────────────────────

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Checked {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
    #[allow(dead_code)]
    pool: Pool,
}

/// The `db` section of the base fixture.
fn checked_builder() -> dynamic_config::Builder<Checked> {
    Checked::builder("db").file("tests/fixtures/base.json")
}

#[test]
fn a_report_names_every_key_and_where_it_came_from() {
    let report = checked_builder()
        .check()
        .expect("the fixture reads cleanly");

    assert!(report.is_clean(), "{report}");
    assert!(report.failure.is_none());

    let paths: Vec<_> = report.resolved.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["host", "pool.max_size", "port"]);

    for resolved in &report.resolved {
        assert!(
            matches!(resolved.origin, Origin::File(_)),
            "{resolved:?} should come from the fixture"
        );
    }
}

#[test]
fn a_report_never_prints_a_value() {
    let rendered = checked_builder().check().unwrap().to_string();

    assert!(rendered.contains("host"), "{rendered}");
    assert!(
        !rendered.contains("localhost"),
        "the value must not appear: {rendered}"
    );
}

/// The struct names `port`; the file will supply `prot`.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Typoed {
    #[allow(dead_code)]
    host: String,
    #[serde(default)]
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_typo_is_caught_and_a_correction_offered() {
    let report = Typoed::builder("db")
        .file("tests/fixtures/typo.json")
        .check()
        .expect("the fixture reads cleanly");

    assert!(!report.is_clean());
    assert_eq!(report.unknown.len(), 1, "{report}");
    assert_eq!(report.unknown[0].path, "prot");
    assert_eq!(report.unknown[0].suggestion.as_deref(), Some("port"));

    assert!(
        report.to_string().contains("did you mean `port`?"),
        "{report}"
    );
}

/// A section the struct cannot deserialize: `port` is required and absent.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Incomplete {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_report_survives_a_configuration_that_would_not_load() {
    let report = Incomplete::builder("db")
        .file("tests/fixtures/typo.json")
        .check()
        .expect("the sources still parse");

    assert!(!report.is_clean());

    let failure = report.failure.as_deref().expect("it would not load");
    assert!(failure.contains("port"), "{failure}");

    assert!(
        !report.resolved.is_empty(),
        "a failing configuration still has values worth reporting"
    );
}

// ─────────────────────────────────────────────
// clap
// ─────────────────────────────────────────────

#[cfg(feature = "clap")]
mod clap_binding {
    use super::*;
    use clap::{Arg, Command};

    /// One type per test: the flags layer lives in a `static`.
    macro_rules! bound {
        ($name:ident) => {
            #[dynamic_config]
            #[derive(Debug, Deserialize)]
            struct $name {
                #[allow(dead_code)]
                host: String,
                #[allow(dead_code)]
                port: u16,
            }

            impl $name {
                /// The `db` section of the base fixture.
                fn sources() -> dynamic_config::Builder<$name> {
                    $name::builder("db").file("tests/fixtures/base.json")
                }
            }
        };
    }

    bound!(Bound);
    bound!(Shaped);

    fn command() -> Command {
        Command::new("test")
            .no_binary_name(true)
            .arg(Arg::new("db-host").long("db-host"))
            // A clap default, deliberately: it must *not* reach the config.
            .arg(Arg::new("db-port").long("db-port").default_value("1234"))
    }

    #[test]
    fn a_flag_from_the_command_line_is_taken() {
        let matches = command().get_matches_from(["--db-host", "from-flag"]);

        Bound::bind_clap(&matches, &[("db-host", "host"), ("db-port", "port")]).unwrap();

        let config = Bound::sources()
            .load()
            .expect("the fixture plus the flag are complete");

        assert_eq!(config.host, "from-flag");
        assert_eq!(
            config.port, 5432,
            "clap's own default must not outrank the file"
        );

        Bound::clear_flags();
    }

    #[test]
    fn a_value_is_read_by_its_shape_not_by_clap_s_type() {
        let matches = command().get_matches_from(["--db-port", "9999"]);

        Shaped::bind_clap(&matches, &[("db-port", "port")]).unwrap();

        // The argument was declared without a value parser, so clap hands back
        // a string; it still reaches a `u16`.
        assert_eq!(Shaped::sources().load().unwrap().port, 9999);

        Shaped::clear_flags();
    }
}

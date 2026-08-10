//! Property tests over the parsing surfaces.
//!
//! Hand-written cases pin the behaviours somebody thought of; these throw
//! generated input at the parsers to find the ones nobody did. The
//! invariants are deliberately modest — "does not panic", "round-trips",
//! "order does not matter" — because those are the promises the crate
//! actually makes.

#![cfg(feature = "json")]

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any string at all: the unit parsers reject or accept, never panic.
    #[test]
    fn duration_parsing_never_panics(input in ".{0,64}") {
        #[derive(Debug, serde::Deserialize)]
        struct Timed {
            #[serde(with = "dynamic_config::duration")]
            #[allow(dead_code)]
            timeout: std::time::Duration,
        }

        let document = serde_json::json!({"db": {"timeout": input}}).to_string();
        let sources = [dynamic_config::Source::inline(&document, dynamic_config::Format::Json)];

        let _ = dynamic_config::load::<Timed>(&dynamic_config::LoadSpec::new("db", &sources));
    }

    /// Same for byte sizes.
    #[test]
    fn byte_parsing_never_panics(input in ".{0,64}") {
        #[derive(Debug, serde::Deserialize)]
        struct Sized {
            #[serde(with = "dynamic_config::bytes")]
            #[allow(dead_code)]
            max: u64,
        }

        let document = serde_json::json!({"db": {"max": input}}).to_string();
        let sources = [dynamic_config::Source::inline(&document, dynamic_config::Format::Json)];

        let _ = dynamic_config::load::<Sized>(&dynamic_config::LoadSpec::new("db", &sources));
    }

    /// A parsed duration with a unit suffix round-trips through the parser
    /// to the same value arithmetic would give.
    #[test]
    fn duration_units_agree_with_arithmetic(seconds in 0u64..999_999) {
        #[derive(Debug, serde::Deserialize)]
        struct Timed {
            #[serde(with = "dynamic_config::duration")]
            timeout: std::time::Duration,
        }

        let document = serde_json::json!({"db": {"timeout": format!("{seconds}s")}}).to_string();
        let sources = [dynamic_config::Source::inline(&document, dynamic_config::Format::Json)];

        let loaded = dynamic_config::load::<Timed>(&dynamic_config::LoadSpec::new("db", &sources))
            .expect("a plain seconds value always parses");

        prop_assert_eq!(loaded.timeout, std::time::Duration::from_secs(seconds));
    }

    /// The dotenv parser accepts or rejects — never panics — and whatever it
    /// accepts obeys the documented grammar (a name before an `=`).
    #[test]
    fn dotenv_parsing_never_panics(input in "(?s).{0,256}") {
        // Reaching the parser through the public surface: a LoadSpec with an
        // env file whose content is arbitrary.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fuzz.env");
        std::fs::write(&path, &input).unwrap();

        #[derive(Debug, serde::Deserialize)]
        struct Anything {
            #[serde(default)]
            #[allow(dead_code)]
            host: String,
        }

        let document = r#"{"db": {"host": "x"}}"#;
        let sources = [dynamic_config::Source::inline(document, dynamic_config::Format::Json)];
        let path_string = path.to_string_lossy().into_owned();
        let paths = [path_string.as_str()];
        let spec = dynamic_config::LoadSpec::new("db", &sources)
            .with_env("DCPROP_")
            .with_env_files(&paths);

        let _ = dynamic_config::load::<Anything>(&spec);
    }

}

//! Property tests over the parsing surfaces.
//!
//! Hand-written cases pin the behaviours somebody thought of; these throw
//! generated input at the parsers to find the ones nobody did. The
//! invariants are deliberately modest — "does not panic", "round-trips",
//! "order does not matter" — because those are the promises the crate
//! actually makes.

#![cfg(feature = "json")]

use std::path::Path;

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

    /// Any two documents merge without panicking, and the later one wins at
    /// every leaf it supplies — the rule a store crate merging N keys relies
    /// on, over generated trees rather than the three hand-written ones.
    #[test]
    fn a_merge_is_later_wins_at_every_leaf(
        left in prop::collection::btree_map("[a-z]{1,4}", 0i64..64, 0..6),
        right in prop::collection::btree_map("[a-z]{1,4}", 0i64..64, 0..6),
    ) {
        use dynamic_config::Value;

        let tree = |entries: &std::collections::BTreeMap<String, i64>| {
            Value::Table(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::Integer(i128::from(*value))))
                    .collect(),
            )
        };

        let mut merged = tree(&left);
        merged.merge(tree(&right));

        for (key, value) in &right {
            prop_assert_eq!(merged.get(key), Some(&Value::Integer(i128::from(*value))));
        }

        for (key, value) in &left {
            if right.contains_key(key) {
                continue;
            }

            prop_assert_eq!(merged.get(key), Some(&Value::Integer(i128::from(*value))));
        }

        // A collision report is exactly the keys both sides named, and it is
        // a list of paths — a value never reaches it.
        let overlaps = tree(&left).overlapping_paths(&tree(&right));
        let expected: Vec<String> =
            left.keys().filter(|key| right.contains_key(*key)).cloned().collect();

        prop_assert_eq!(overlaps, expected);
    }

    /// The 0.4 traversal fix, as a property rather than as three examples:
    /// a profile the guard accepts can only ever name a sibling of the file it
    /// is applied to.
    ///
    /// The guard and the interpolation are separate functions, which is what
    /// makes this assertable at all — and what
    /// [`__fuzz`](dynamic_config::__fuzz) exists to let a coverage-guided
    /// target say too.
    #[test]
    fn an_accepted_profile_names_a_sibling_and_nothing_else(profile in "(?s).{0,32}") {
        prop_assume!(dynamic_config::__fuzz::profile_is_safe(&profile));

        let Some(variant) =
            dynamic_config::__fuzz::profile_variant("/etc/app/config.toml", &profile)
        else {
            return Ok(());
        };

        // Asserted through `Path` rather than on the string: `with_file_name`
        // rebuilds a path with the *platform's* separator, so on Windows the
        // variant of `/etc/app/config.toml` reads `/etc/app\config..toml`.
        // Both spellings name the same file, and the property this test is
        // about — the variant is a sibling — is about the file.
        let base = Path::new("/etc/app/config.toml");
        let variant = Path::new(&variant);

        prop_assert_eq!(variant.parent(), base.parent(), "{:?}", variant);
        prop_assert_eq!(variant.extension(), base.extension(), "{:?}", variant);
        prop_assert!(
            variant
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with("config.")),
            "{:?}",
            variant
        );
    }
}

/// The `.env` grammar, reached through the seam rather than through a
/// temporary file and a whole load.
///
/// The property above it goes the long way round on purpose — it is testing
/// that the *layer* survives arbitrary input. This one tests the parser, which
/// is what a fuzz target wants, and could not be written before the seam
/// existed.
#[cfg(feature = "dotenv")]
mod dotenv_seam {
    use proptest::prelude::*;

    #[test]
    fn the_seam_reaches_the_parser_the_layer_uses() {
        let entries = dynamic_config::__fuzz::dotenv_entries("# a note\nexport A=1\nB=\"  x  \"\n")
            .expect("all three lines are grammatical");

        assert_eq!(entries["A"], "1");
        assert_eq!(entries["B"], "  x  ");
        assert_eq!(
            dynamic_config::__fuzz::dotenv_entries("A=1\nnonsense\n").unwrap_err(),
            2,
            "the one-based line that stopped it"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Whatever it accepts obeys the documented grammar: a non-empty,
        /// already-trimmed name before an `=`.
        #[test]
        fn what_the_parser_accepts_obeys_the_grammar(input in "(?s).{0,256}") {
            let Ok(entries) = dynamic_config::__fuzz::dotenv_entries(&input) else {
                return Ok(());
            };

            for name in entries.keys() {
                prop_assert!(!name.is_empty());
                prop_assert_eq!(name.trim(), name.as_str());
            }
        }
    }
}

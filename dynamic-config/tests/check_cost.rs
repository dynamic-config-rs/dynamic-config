//! `check()` must build the configuration once, not once per key.
//!
//! Pinned with a counting provider: figment calls `data()` when a provider is
//! merged, so the count *is* the number of builds.

#![cfg(all(feature = "figment", feature = "json"))]

use std::sync::atomic::{AtomicUsize, Ordering};

use dynamic_config::figment::value::{Dict, Value};
use dynamic_config::figment::{Metadata, Profile, Provider};
use dynamic_config::{check, LoadSpec, Source};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Wide {
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    f: u8,
    g: u8,
    h: u8,
}

/// Counts how many times figment asks it for data.
struct Counting(AtomicUsize);

impl Provider for Counting {
    fn metadata(&self) -> Metadata {
        Metadata::named("a counting provider")
    }

    fn data(
        &self,
    ) -> dynamic_config::figment::Result<dynamic_config::figment::value::Map<Profile, Dict>> {
        self.0.fetch_add(1, Ordering::SeqCst);

        let mut section = Dict::new();

        for (index, name) in ["a", "b", "c", "d", "e", "f", "g", "h"].iter().enumerate() {
            section.insert((*name).to_owned(), Value::from(index as u8));
        }

        let mut map = dynamic_config::figment::value::Map::new();
        map.insert(Profile::from("db"), section);

        Ok(map)
    }
}

#[test]
fn a_report_costs_a_constant_number_of_builds() {
    let provider = Counting(AtomicUsize::new(0));
    let sources = [Source::provider(&provider)];
    let spec = LoadSpec::new("db", &sources);

    let report = check::<Wide>(&spec, &["a", "b", "c", "d", "e", "f", "g", "h"])
        .expect("the provider resolves");

    assert_eq!(report.resolved.len(), 8, "every key is reported");

    let builds = provider.0.load(Ordering::SeqCst);

    // One build for the report, one deliberate `load` for `failure` (so the
    // error names the file at fault). The old shape was 2 + one per key.
    assert!(
        builds <= 2,
        "check() rebuilt the configuration {builds} times for 8 keys — \
         it must build once, not once per key"
    );
}

/// A provider that says where its values came from, the way
/// `Source::provider`'s documentation tells one to.
struct Described(std::path::PathBuf);

impl Provider for Described {
    fn metadata(&self) -> Metadata {
        // Both halves: the name reaches messages, the *source* reaches
        // provenance. The documentation for `Source::provider` spends a
        // paragraph on exactly this distinction.
        Metadata::from("INI file", self.0.as_path())
    }

    fn data(
        &self,
    ) -> Result<dynamic_config::figment::value::Map<Profile, Dict>, dynamic_config::figment::Error>
    {
        let mut section = Dict::new();
        section.insert("a".to_owned(), Value::from(1_u8));

        let mut map = dynamic_config::figment::value::Map::new();
        map.insert(Profile::from("db"), section);

        Ok(map)
    }
}

/// **A provider's metadata source is its values' provenance.**
///
/// Every foreign provider used to answer `Origin::Inline`, however
/// carefully it described itself — so a provider doing exactly what the
/// documentation asks got a diagnostic pointing at nothing. The name is
/// still only a name: `Metadata::named(..)` alone leaves the origin
/// unknown, which is what the same paragraph promises.
#[test]
fn a_provider_that_names_its_file_is_traced_back_to_it() {
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct One {
        a: u8,
    }

    let path = std::path::PathBuf::from("/etc/myapp/db.ini");
    let described = Described(path.clone());
    let sources = [Source::provider(&described)];

    let report = check::<One>(&LoadSpec::new("db", &sources), &["a"]).expect("it resolves");
    let origin = report
        .resolved
        .iter()
        .find(|entry| entry.path == "a")
        .map(|entry| entry.origin.clone())
        .expect("`a` is reported");

    assert_eq!(
        origin,
        dynamic_config::Origin::File(path),
        "the provider's own metadata says where the value lives"
    );

    // Named but not sourced: unknown, rather than a file nobody named.
    struct NamedOnly;

    impl Provider for NamedOnly {
        fn metadata(&self) -> Metadata {
            Metadata::named("an INI file somewhere")
        }

        fn data(
            &self,
        ) -> Result<
            dynamic_config::figment::value::Map<Profile, Dict>,
            dynamic_config::figment::Error,
        > {
            Described(std::path::PathBuf::new()).data()
        }
    }

    let anonymous = NamedOnly;
    let sources = [Source::provider(&anonymous)];
    let report = check::<One>(&LoadSpec::new("db", &sources), &["a"]).expect("it resolves");

    assert_eq!(
        report
            .resolved
            .iter()
            .find(|entry| entry.path == "a")
            .map(|entry| entry.origin.clone()),
        Some(dynamic_config::Origin::Unknown),
        "a name is not a source"
    );
}

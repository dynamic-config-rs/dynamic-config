//! A section may be called anything a key may be called.
//!
//! Sections used to be the backend's profiles, and that backend reserves two
//! profile names with inheritance rules of their own. A section is now the
//! subtree under its key, so the reserved words are ordinary keys again —
//! asserted here rather than assumed, because the reason they were safe
//! before was a prefix this crate no longer applies.

#![cfg(feature = "json")]

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Server {
    port: u16,
}

fn write(name: &str, contents: &str) -> String {
    let directory = std::env::temp_dir().join("dynamic-config-reserved");
    std::fs::create_dir_all(&directory).expect("the scratch directory is creatable");

    let path = directory.join(name);
    std::fs::write(&path, contents).expect("the fixture is writable");

    path.display().to_string()
}

#[test]
fn a_section_may_be_called_global_or_default() {
    let file = write(
        "reserved.json",
        r#"{"global": {"port": 1}, "default": {"port": 2}, "other": {"port": 3}}"#,
    );

    for (section, expected) in [("global", 1), ("default", 2), ("other", 3)] {
        let config: Server = dynamic_config::Builder::new(section)
            .file(file.clone())
            .load()
            .expect("the section reads like any other");

        assert_eq!(config.port, expected, "section `{section}`");
    }
}

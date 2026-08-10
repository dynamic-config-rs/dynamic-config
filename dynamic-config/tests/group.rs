//! Several configuration types reloading as one step.

#![cfg(feature = "json")]

use dynamic_config::{dynamic_config, ReloadGroup};
use serde::Deserialize;

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "server")]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
}

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db")]
#[derive(Debug, Deserialize)]
struct DbConfig {
    #[allow(dead_code)]
    host: String,
    port: u16,
}

#[test]
fn a_group_installs_every_member() {
    let group = ReloadGroup::new().with::<ServerConfig>().with::<DbConfig>();

    assert_eq!(
        group.members().collect::<Vec<_>>(),
        ["ServerConfig", "DbConfig"]
    );

    group.reload().expect("the fixture is complete");

    assert_eq!(ServerConfig::current().port, 8080);
    assert_eq!(DbConfig::current().port, 5432);
}

/// The property the group exists for: one member failing leaves *every*
/// member on its previous snapshot, including the ones that loaded cleanly.
#[dynamic_config(files = ["tests/fixtures/base.json"], key = "server")]
#[derive(Debug, Deserialize)]
struct Healthy {
    port: u16,
}

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "server")]
#[derive(Debug, Deserialize)]
struct Fragile {
    port: u16,
}

#[test]
fn one_failure_leaves_every_member_where_it_was() {
    let group = ReloadGroup::new().with::<Healthy>().with::<Fragile>();

    group.reload().expect("both start out loadable");
    assert_eq!(Healthy::current().port, 8080);
    assert_eq!(Fragile::current().port, 8080);

    // Break only the second member, and change the first at the same time —
    // the way one edit to a shared file would.
    Healthy::set_override("port", 9999u16).unwrap();
    Fragile::set_override("port", "not-a-number").unwrap();

    let error = group.reload().expect_err("`Fragile` cannot load");
    assert!(error.path().starts_with("Fragile"), "{error}");

    assert_eq!(
        Healthy::current().port,
        8080,
        "the member that loaded cleanly must not have been installed either"
    );
    assert_eq!(Fragile::current().port, 8080);

    // And once the break is repaired, both move together.
    Fragile::clear_overrides();
    group.reload().expect("both load again");

    assert_eq!(Healthy::current().port, 9999);
    assert_eq!(Fragile::current().port, 8080);

    Healthy::clear_overrides();
}

//! Trybuild's scratch crate has no `decrypt` feature of its own — exactly the
//! situation of every real user. `save_encrypted` must be callable anyway,
//! because the feature that gates it belongs to dynamic-config, not to the
//! caller — and the generated surface next to it must compile in a crate with
//! an empty feature set.

use dynamic_config::dynamic_config;
use serde::{Deserialize, Serialize};

#[dynamic_config]
#[derive(Deserialize, Serialize)]
struct AppConfig {
    name: String,
}

#[allow(dead_code)]
fn exercises_the_saving_surface(
    config: &AppConfig,
    encryptor: &dyn dynamic_config::Encryptor,
) -> Result<(), dynamic_config::Error> {
    dynamic_config::save_encrypted(
        config,
        "secrets.json.age",
        dynamic_config::Format::Json,
        "app",
        encryptor,
    )
}

fn main() {}

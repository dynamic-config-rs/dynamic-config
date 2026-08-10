//! Trybuild's scratch crate has no `decrypt` feature of its own — exactly the
//! situation of every real user. `save_encrypted` must appear anyway, because
//! the feature that gates it belongs to dynamic-config, not to the caller.
//!
//! Before the fix this failed to compile: the generated method carried
//! `#[cfg(feature = "decrypt")]`, which resolved against this crate's (empty)
//! feature set and compiled the method away.

use dynamic_config::dynamic_config;
use serde::{Deserialize, Serialize};

#[dynamic_config(files = ["app.json"], key = "app", save)]
#[derive(Deserialize, Serialize)]
struct AppConfig {
    name: String,
}

#[allow(dead_code)]
fn exercises_the_generated_surface(
    config: &AppConfig,
    encryptor: &dyn dynamic_config::Encryptor,
) -> Result<(), dynamic_config::Error> {
    config.save_encrypted("secrets.json.age", encryptor)
}

fn main() {}

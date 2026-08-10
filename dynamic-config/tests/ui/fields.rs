//! Rejections that come from the field attributes rather than the arguments.

use dynamic_config::dynamic_config;
use serde::Deserialize;

// Both would generate a `Debug`, and the derived one would win the race to
// print the secret.
#[dynamic_config(files = ["a.json"], key = "a")]
#[derive(Debug, Deserialize)]
struct DebugAndSecret {
    #[config(secret)]
    password: String,
}

#[dynamic_config(files = ["a.json"], key = "a")]
#[derive(Deserialize)]
struct UnknownOption {
    #[config(sercet)]
    password: String,
}

fn main() {}

//! An encrypted file without decryption support.

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(files = ["secrets.json.age"], key = "db")]
#[derive(Deserialize)]
struct NoFeature {
    x: u8,
}

fn main() {}

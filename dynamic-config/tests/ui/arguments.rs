//! The attribute takes no arguments any more: anything between the
//! parentheses gets the migration map, not a parse of the old grammar.

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(files = ["a.json"], key = "db")]
#[derive(Deserialize)]
struct StillOldStyle {
    x: u8,
}

fn main() {}

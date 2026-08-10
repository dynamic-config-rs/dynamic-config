//! Configuration with no files at all — the 12-factor arrangement.
//!
//! A section that no file provides is an empty table rather than an error, so
//! the environment can supply everything.
//!
//! Run from the workspace root:
//!
//! ```text
//! APP_WORKER_CONCURRENCY=8 APP_WORKER_QUEUES=high,default,low \
//!   APP_WORKER_RETRY__MAX_ATTEMPTS=5 \
//!   cargo run -p dynamic-config --example env_only
//! ```

use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config(files = ["dynamic-config/examples/config.json"], key = "worker", env = "APP_")]
#[derive(Debug, Deserialize)]
// Read through the `Debug` impl, which dead-code analysis ignores.
#[allow(dead_code)]
struct WorkerConfig {
    /// `APP_WORKER_CONCURRENCY=8` — a string in the environment, a number here.
    concurrency: u16,
    /// `APP_WORKER_QUEUES=high,default,low` — commas make a list.
    queues: Vec<String>,
    /// `APP_WORKER_RETRY__MAX_ATTEMPTS=5` — a doubled underscore nests.
    retry: RetryConfig,
}

#[derive(Debug, Deserialize)]
// Read through the `Debug` impl, which dead-code analysis ignores.
#[allow(dead_code)]
struct RetryConfig {
    max_attempts: u8,
}

fn main() {
    match WorkerConfig::load() {
        Ok(config) => println!("{config:#?}"),
        // The error names the field and the variable that should have set it.
        Err(error) => println!("could not load: {error}"),
    }
}

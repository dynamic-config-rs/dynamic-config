//! Rejecting a configuration in which every field is fine and the whole is not.
//!
//! ```text
//! cargo run -p dynamic-config --example validation --features json
//! ```

use dynamic_config::{dynamic_config, ErrorKind};
use serde::Deserialize;

#[dynamic_config(files = ["dynamic-config/examples/pool.json"], key = "pool", validate)]
#[derive(Debug, Deserialize)]
struct PoolConfig {
    min_size: u16,
    max_size: u16,
    idle_timeout_secs: u64,
}

impl PoolConfig {
    /// Resolved at this call site by the generated `load`, so it can be an
    /// inherent method, a `validator::Validate` impl, a `garde` one — anything
    /// with a `validate(&self) -> Result<_, impl Display>`.
    fn validate(&self) -> Result<(), String> {
        if self.min_size > self.max_size {
            return Err(format!(
                "min_size ({}) is above max_size ({})",
                self.min_size, self.max_size
            ));
        }

        if self.max_size > 0 && self.idle_timeout_secs == 0 {
            return Err("a pool that never idles out will hold connections forever".to_owned());
        }

        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    PoolConfig::init()?;
    println!("loaded: {:?}", PoolConfig::current());

    // Every field below still parses as a `u16`. Only the relationship between
    // them is wrong, which no `Deserialize` impl can see.
    PoolConfig::set_override("min_size", 100u16)?;

    match PoolConfig::load() {
        Ok(config) => println!("unexpectedly loaded {config:?}"),
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::Invalid);
            println!("\nrejected: {error}");
        }
    }

    // The snapshot is untouched: a failed validation keeps the previous
    // configuration exactly as a failed parse does. A bad edit degrades to
    // "no change", never to a crash and never to a half-applied state.
    println!("still serving: {:?}", PoolConfig::current());

    PoolConfig::clear_overrides();

    Ok(())
}

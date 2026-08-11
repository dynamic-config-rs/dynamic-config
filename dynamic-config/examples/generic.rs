//! One configuration struct, several independent snapshots.
//!
//! ```text
//! cargo run -p dynamic-config --example generic --features json
//! ```
//!
//! `Db<Postgres>` and `Db<Mysql>` are different types, so they get different
//! snapshots and different runtime layers. Rust has no generic statics, so this
//! goes through a `TypeId` registry — about 10 ns more per read than the
//! `static` a non-generic type gets. `benches/read_path.rs` measures both.

use std::marker::PhantomData;

use dynamic_config::dynamic_config;
use serde::Deserialize;

trait Driver {
    const SCHEME: &'static str;
}

struct Postgres;
struct Mysql;

impl Driver for Postgres {
    const SCHEME: &'static str = "postgres";
}

impl Driver for Mysql {
    const SCHEME: &'static str = "mysql";
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Db<D: Driver> {
    host: String,
    port: u16,
    // `fn() -> D` rather than `D`, so the marker is `Send + Sync` whatever `D`
    // is — the usual shape for a type-level tag.
    #[serde(skip)]
    driver: PhantomData<fn() -> D>,
}

impl<D: Driver> Db<D> {
    fn url(&self) -> String {
        format!("{}://{}:{}", D::SCHEME, self.host, self.port)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The same sources twice: each instantiation is its own configuration,
    // so each gets its own builder.
    let postgres = Db::<Postgres>::builder("db").file("dynamic-config/examples/databases.json");
    let mysql = Db::<Mysql>::builder("db").file("dynamic-config/examples/databases.json");

    postgres.init()?;
    mysql.init()?;

    println!("postgres: {}", Db::<Postgres>::current().url());
    println!("mysql:    {}", Db::<Mysql>::current().url());

    // Each instantiation owns its layers too, so this reaches exactly one.
    Db::<Mysql>::set_override("port", 3306u16)?;

    println!("\nafter overriding only the mysql port:");
    println!("  postgres: {}", postgres.load()?.url());
    println!("  mysql:    {}", mysql.load()?.url());

    Db::<Mysql>::clear_overrides();

    println!("\nA lifetime parameter would be rejected at compile time: the");
    println!("snapshot outlives every borrow that could name one.");

    Ok(())
}

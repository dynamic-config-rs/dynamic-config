//! A command-line front end: flags over the environment, `--check` instead of
//! booting.
//!
//! ```text
//! cargo run -p dynamic-config --example cli --features json,clap -- --check
//! cargo run -p dynamic-config --example cli --features json,clap -- --port 9000
//! cargo run -p dynamic-config --example cli --features json,clap -- --set host=0.0.0.0
//! ```
//!
//! Keys are relative to the section this struct maps to, so it is `host`, not
//! `server.host`: each configuration type owns its own flags layer, exactly as
//! it owns its own files and environment prefix.

use clap::{Arg, ArgAction, Command};
use dynamic_config::dynamic_config;
use serde::Deserialize;

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    #[allow(dead_code)]
    tags: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = Command::new("cli")
        .arg(Arg::new("port").long("port").help("override server.port"))
        .arg(
            Arg::new("set")
                .long("set")
                .action(ArgAction::Append)
                .help("key=value relative to the section, repeatable"),
        )
        .arg(
            Arg::new("check")
                .long("check")
                .action(ArgAction::SetTrue)
                .help("report the configuration and exit"),
        )
        .get_matches();

    // Named arguments first, then the escape hatch for anything without a flag
    // of its own. Both land in the same layer, above the environment.
    ServerConfig::bind_clap(&matches, &[("port", "port")])?;

    if let Some(assignments) = matches.get_many::<String>("set") {
        ServerConfig::set_assignments(assignments)?;
    }

    // Sources are chosen at runtime; the flags layer bound above sits over
    // whatever these resolve to.
    let sources = ServerConfig::builder("server")
        .file("dynamic-config/examples/config.json")
        .env("APP_");

    // `--check` has to work when the configuration is broken, which is exactly
    // when it is worth running — so it reports rather than loading.
    if matches.get_flag("check") {
        let report = sources.check()?;

        println!("{report}");

        return if report.is_clean() {
            Ok(())
        } else {
            Err("configuration is not clean".into())
        };
    }

    sources.init()?;

    let config = ServerConfig::current();
    println!("listening on {}:{}", config.host, config.port);

    Ok(())
}

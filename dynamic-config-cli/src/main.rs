//! Command-line diagnostics for `dynamic-config`.
//!
//! A CLI cannot see an application's `#[dynamic_config]` attribute, so it
//! builds the load from flags instead — `explain` is the command-line form
//! of `check()`, pointed at one path. What the flags describe must match
//! what the application declares, or the answer is about a different load;
//! the flags exist because that is the honest boundary, not a limitation to
//! engineer away.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use dynamic_config::{explain, snapshot, Format, LoadSpec, Source};

#[derive(Parser)]
#[command(name = "dynamic-config", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Every layer's answer for one path, not just the winner's.
    ///
    /// The output contains values — that is its point. Pass --secret for a
    /// path you know to be sensitive and every value prints as ***.
    Explain {
        /// Dotted path, relative to the section: pool.max_size
        path: String,
        /// Configuration file, repeatable; merged left to right, later wins
        #[arg(short, long = "file", required = true)]
        files: Vec<String>,
        /// The section the struct maps to: db
        #[arg(short, long)]
        key: String,
        /// Environment variable prefix: APP_
        #[arg(short, long)]
        env: Option<String>,
        /// Environment variable naming the active profile: APP_ENV
        #[arg(long)]
        profile_env: Option<String>,
        /// .env file read as the environment layer, repeatable
        #[arg(long = "env-file")]
        env_files: Vec<String>,
        /// Print every value as *** — for a path you know is sensitive
        #[arg(long)]
        secret: bool,
    },
    /// Which paths differ between two documents. Paths only, never values.
    Diff {
        /// The earlier document
        old: String,
        /// The later document
        new: String,
        /// The section to compare: db
        #[arg(short, long)]
        key: String,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), dynamic_config::Error> {
    match cli.command {
        Command::Explain {
            path,
            files,
            key,
            env,
            profile_env,
            env_files,
            secret,
        } => {
            let sources = sources(&files)?;
            let env_files: Vec<&str> = env_files.iter().map(String::as_str).collect();

            let mut spec = LoadSpec::new(&key, &sources).with_env_files(&env_files);
            if let Some(prefix) = &env {
                spec = spec.with_env(prefix);
            }
            if let Some(variable) = &profile_env {
                spec = spec.with_profile_env(variable);
            }

            let mut explanation = explain(&spec, &path)?;
            if secret {
                explanation = explanation.redacted();
            }

            print!("{explanation}");
            Ok(())
        }
        Command::Diff { old, new, key } => {
            let before = one_document(&old, &key)?;
            let after = one_document(&new, &key)?;

            for change in before.diff(&after) {
                println!("{change}");
            }

            Ok(())
        }
    }
}

/// The listed files as sources, formats taken from their extensions.
fn sources(files: &[String]) -> Result<Vec<Source<'_>>, dynamic_config::Error> {
    files
        .iter()
        .map(|file| {
            Format::from_path(std::path::Path::new(file)).map(|format| Source::file(file, format))
        })
        .collect()
}

/// One file's section, resolved on its own.
fn one_document(file: &str, key: &str) -> Result<dynamic_config::Snapshot, dynamic_config::Error> {
    let format = Format::from_path(std::path::Path::new(file))?;
    let sources = [Source::file(file, format)];

    snapshot(&LoadSpec::new(key, &sources))
}

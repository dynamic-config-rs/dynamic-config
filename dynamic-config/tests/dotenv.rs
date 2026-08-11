//! `.env` files, read as the environment rather than as documents.

#![cfg(all(feature = "dotenv", feature = "json"))]

use std::path::PathBuf;

use dynamic_config::{dynamic_config, Builder};
use serde::Deserialize;

fn scratch(test: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("dynamic-config-dotenv")
        .join(test);

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();

    directory
}

// A type *and* a file per test. Each builder points at one fixed scratch path,
// so two tests sharing one write over each other — in parallel, intermittently,
// which is the worst way to find out.
#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Basic {
    host: String,
    port: u16,
    pool: Pool,
}

/// The base fixture plus this test's own `.env` scratch file.
fn basic_builder() -> Builder<Basic> {
    Basic::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCENV_")
        .env_file("tests/scratch/dotenv-basic.env")
}

#[derive(Debug, Deserialize)]
struct Pool {
    max_size: u16,
}

fn write(path: &str, contents: &str) {
    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn a_dotenv_file_supplies_the_environment_layer() {
    write(
        "tests/scratch/dotenv-basic.env",
        "# a note\nDCENV_DB_HOST=from-the-file\nDCENV_DB_POOL__MAX_SIZE=64\n",
    );

    let config = basic_builder().load().expect("the file parses");

    assert_eq!(config.host, "from-the-file", "the prefix is stripped");
    assert_eq!(
        config.pool.max_size, 64,
        "and the nesting separator applies"
    );
    assert_eq!(
        config.port, 5432,
        "everything else still comes from the file"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Quiet {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

/// The base fixture plus this test's own `.env` scratch file.
fn quiet_builder() -> Builder<Quiet> {
    Quiet::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCENVQ_")
        .env_file("tests/scratch/dotenv-quiet.env")
}

#[test]
fn it_does_not_touch_the_process_environment() {
    write(
        "tests/scratch/dotenv-quiet.env",
        "DCENVQ_DB_HOST=from-the-file\n",
    );

    quiet_builder().load().expect("the file parses");

    assert!(
        std::env::var("DCENVQ_DB_HOST").is_err(),
        "reading a `.env` must not change the environment of the whole process"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Precedence {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

/// The base fixture plus this test's own `.env` scratch file.
fn precedence_builder() -> Builder<Precedence> {
    Precedence::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCENVP_")
        .env_file("tests/scratch/dotenv-precedence.env")
}

#[test]
fn a_real_variable_beats_the_file() {
    write(
        "tests/scratch/dotenv-precedence.env",
        "DCENVP_DB_HOST=from-the-file\n",
    );

    assert_eq!(precedence_builder().load().unwrap().host, "from-the-file");

    std::env::set_var("DCENVP_DB_HOST", "from-the-machine");

    assert_eq!(
        precedence_builder().load().unwrap().host,
        "from-the-machine",
        "a variable exported for this run beats a file in the repository"
    );

    std::env::remove_var("DCENVP_DB_HOST");
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Traced {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

#[test]
fn a_value_from_a_dotenv_file_is_traced_to_that_file() {
    write(
        "tests/scratch/dotenv-traced.env",
        "DCENVT_DB_HOST=from-the-file\n",
    );

    // No init has happened, so the question goes to the builder directly.
    let origin = Traced::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCENVT_")
        .env_file("tests/scratch/dotenv-traced.env")
        .source_of("host")
        .unwrap()
        .expect("something supplies it");

    assert!(
        format!("{origin}").contains("dotenv-traced.env"),
        "not `the environment` in general: {origin}"
    );
}

#[dynamic_config]
#[derive(Debug, Deserialize)]
struct Missing {
    host: String,
    #[allow(dead_code)]
    port: u16,
}

/// The base fixture plus a `.env` path this test makes sure does not exist.
fn missing_builder() -> Builder<Missing> {
    Missing::builder("db")
        .file("tests/fixtures/base.json")
        .env("DCENVM_")
        .env_file("tests/scratch/dotenv-absent.env")
}

#[test]
fn a_file_that_is_not_there_is_skipped_like_any_other() {
    let _ = std::fs::remove_file("tests/scratch/dotenv-absent.env");

    assert_eq!(
        missing_builder()
            .load()
            .expect("an absent `.env` is not an error")
            .host,
        "localhost",
        "an optional `.env` next to a deployment that sets real variables"
    );
}

#[test]
fn a_malformed_line_names_the_file_and_the_line() {
    use dynamic_config::{ErrorKind, Format, LoadSpec, Source};

    let directory = scratch("malformed");
    let path = directory.join("broken.env");

    std::fs::write(&path, "DCX_DB_HOST=ok\nthis is not an assignment\n").unwrap();

    let sources: [Source; 0] = [];
    let files = [path.to_str().unwrap()];
    let spec = LoadSpec::new("db", &sources)
        .with_env("DCX_")
        .with_env_files(&files);

    let error = dynamic_config::load::<Missing>(&spec).expect_err("line 2 is nonsense");

    assert_eq!(error.kind(), ErrorKind::Parse);
    assert!(error.to_string().contains("line 2"), "{error}");
    assert!(error.to_string().contains("broken.env"), "{error}");

    let _ = Format::Json;
}

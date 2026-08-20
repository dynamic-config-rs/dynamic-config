//! One load, composed by every engine, compared.
//!
//! Which engine folds a load is a choice about whose code runs, never about
//! what the configuration means — so every engine this build has is given
//! the same layers, in the same order, from the same sources, and must
//! answer with the same tree *and* the same winner for every leaf. A
//! disagreement is a bug in whichever adapter drifted, and it shows up here
//! rather than in somebody's deployment.
//!
//! The stacks below are the ones that make composition visible: overlapping
//! keys across layers, tables meeting scalars, sections beside other
//! sections, a profile's sibling file, secrets, `.env` files, the real
//! environment, `--set` and overrides.

#![cfg(all(
    feature = "figment",
    feature = "json",
    feature = "toml",
    feature = "dotenv"
))]

use std::fs;
use std::path::PathBuf;

use dynamic_config::{Format, Layer, LoadSpec, Source};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("dynamic-config-composition")
        .join(name);

    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("the scratch directory is creatable");

    directory
}

/// Every engine's composition of `spec`, which must be the same text — and
/// the same account of who supplied what.
#[track_caller]
fn agree(spec: &LoadSpec<'_>, what: &str) {
    for answers in [
        dynamic_config::__fuzz::compositions(spec),
        dynamic_config::__fuzz::provenances(spec),
    ] {
        let (reference_name, reference) = &answers[0];

        for (name, answer) in &answers[1..] {
            assert_eq!(
                answer, reference,
                "the {reference_name} and {name} engines disagree on {what}"
            );
        }
    }
}

#[test]
fn layers_over_one_another() {
    let directory = scratch("layers");
    let file = directory.join("config.toml");

    fs::write(
        &file,
        "[db]\nhost = \"from-the-file\"\nport = 5432\n\n[other]\nignored = true\n",
    )
    .unwrap();

    let defaults = Layer::new();
    defaults.set("host", "from-the-defaults").unwrap();
    defaults.set("timeout", 30u16).unwrap();

    let flags = Layer::new();
    flags.set_text("port", "6543").unwrap();

    let overrides = Layer::new();
    overrides.set("pool.max", 32u16).unwrap();

    let path = file.display().to_string();
    let sources = [Source::file(&path, Format::Toml)];

    let spec = LoadSpec::new("db", &sources)
        .with_defaults(&defaults)
        .with_flags(&flags)
        .with_overrides(&overrides);

    agree(&spec, "a file under defaults, over flags and overrides");
}

#[test]
fn a_table_and_a_scalar_meeting_at_one_path() {
    let directory = scratch("shapes");
    let first = directory.join("first.json");
    let second = directory.join("second.json");

    fs::write(&first, r#"{"db": {"pool": 10}}"#).unwrap();
    fs::write(&second, r#"{"db": {"pool": {"max": 32}}}"#).unwrap();

    let (one, two) = (first.display().to_string(), second.display().to_string());
    let sources = [
        Source::file(&one, Format::Json),
        Source::file(&two, Format::Json),
    ];

    agree(
        &LoadSpec::new("db", &sources),
        "a table arriving over a scalar",
    );

    let sources = [
        Source::file(&two, Format::Json),
        Source::file(&one, Format::Json),
    ];

    agree(
        &LoadSpec::new("db", &sources),
        "a scalar arriving over a table",
    );
}

#[test]
fn the_whole_document_layout() {
    let directory = scratch("whole");
    let file = directory.join("whole.json");

    fs::write(&file, r#"{"host": "here", "pool": {"max": 8}}"#).unwrap();

    let path = file.display().to_string();
    let sources = [Source::file(&path, Format::Json)];
    let spec = LoadSpec::new("db", &sources).with_whole_document(true);

    agree(&spec, "a document that is one section");
}

#[test]
fn an_absent_file_and_an_empty_one() {
    let directory = scratch("absent");
    let empty = directory.join("empty.json");
    fs::write(&empty, "{}").unwrap();

    let missing = directory.join("missing.json").display().to_string();
    let empty = empty.display().to_string();
    let sources = [
        Source::file(&missing, Format::Json),
        Source::file(&empty, Format::Json),
    ];

    agree(
        &LoadSpec::new("db", &sources),
        "a file that is not there and one that says nothing",
    );
}

#[test]
fn inline_sources_beside_files() {
    let sources = [
        Source::inline(
            r#"{"db": {"host": "inline-one", "tags": ["a"]}}"#,
            Format::Json,
        ),
        Source::inline("[db]\ntags = [\"b\", \"c\"]\n", Format::Toml),
    ];

    agree(
        &LoadSpec::new("db", &sources),
        "two inline documents, arrays replacing",
    );
}

#[test]
fn the_environment_and_a_dotenv_file() {
    let directory = scratch("env");
    let dotenv = directory.join("config.env");

    fs::write(
        &dotenv,
        "DCCOMPOSE_DB_HOST=from-dotenv\nDCCOMPOSE_DB_POOL__MAX=4\n",
    )
    .unwrap();

    std::env::set_var("DCCOMPOSE_DB_HOST", "from-the-environment");
    std::env::set_var("DCCOMPOSE_DB_PORT", "7777");

    let dotenv = dotenv.display().to_string();
    let files = [dotenv.as_str()];
    let sources: [Source<'_>; 0] = [];

    let spec = LoadSpec::new("db", &sources)
        .with_env("DCCOMPOSE_")
        .with_env_files(&files);

    agree(&spec, "the environment over a .env file");

    std::env::remove_var("DCCOMPOSE_DB_HOST");
    std::env::remove_var("DCCOMPOSE_DB_PORT");
}

#[test]
fn a_secrets_directory() {
    let directory = scratch("secrets");
    let secrets = directory.join("secrets");
    fs::create_dir_all(&secrets).unwrap();

    fs::write(secrets.join("db__password"), "hunter2\n").unwrap();
    fs::write(secrets.join("db__pool__max"), "16").unwrap();

    let file = directory.join("config.toml");
    fs::write(&file, "[db]\npassword = \"from-the-file\"\n").unwrap();

    let path = file.display().to_string();
    let secrets = secrets.display().to_string();
    let sources = [Source::file(&path, Format::Toml)];

    let spec = LoadSpec::new("db", &sources).with_secrets_dir(&secrets);

    agree(&spec, "a secrets directory over a file");
}

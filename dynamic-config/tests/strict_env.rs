//! `strict_env`: ambiguous environment spellings are errors, not guesses.

#![cfg(feature = "json")]

use serde::Deserialize;

#[test]
fn an_ambiguous_spelling_is_refused_and_named() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Strict {
        #[allow(dead_code)]
        #[serde(default)]
        mode: String,
    }

    std::env::set_var("STRICTREFUSE_SVC_MODE", "off");

    let error = Strict::builder("svc")
        .env("STRICTREFUSE_")
        .strict_env()
        .load()
        .expect_err("`off` is exactly the ambiguity strict mode exists for");
    let message = error.to_string();

    assert!(message.contains("STRICTREFUSE_SVC_MODE"), "{message}");
    assert!(message.contains("strict_env"), "{message}");

    std::env::remove_var("STRICTREFUSE_SVC_MODE");
}

#[test]
fn an_unambiguous_value_passes_strict_mode() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Strict {
        #[serde(default)]
        mode: String,
        #[serde(default)]
        tls: bool,
    }

    std::env::set_var("STRICTPASS_SVC_MODE", "fast");
    std::env::set_var("STRICTPASS_SVC_TLS", "true");

    let config = Strict::builder("svc")
        .env("STRICTPASS_")
        .strict_env()
        .load()
        .expect("nothing ambiguous is set");
    assert_eq!(config.mode, "fast");
    assert!(config.tls);

    std::env::remove_var("STRICTPASS_SVC_MODE");
    std::env::remove_var("STRICTPASS_SVC_TLS");
}

#[test]
fn without_strict_the_same_spelling_is_accepted_loosely() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Loose {
        #[serde(default)]
        mode: String,
    }

    std::env::set_var("STRICTLOOSE_SVC_MODE", "off");

    let config = Loose::builder("svc")
        .env("STRICTLOOSE_")
        .load()
        .expect("loose is the default, and accepts it");
    assert_eq!(config.mode, "off", "it arrives as the string it always was");

    std::env::remove_var("STRICTLOOSE_SVC_MODE");
}

#[cfg(feature = "dotenv")]
#[test]
fn a_dotenv_file_is_held_to_the_same_standard() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct Strict {
        #[allow(dead_code)]
        #[serde(default)]
        mode: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write("tests/scratch/strict.env", "STRICTDOTENV_SVC_MODE=no\n").unwrap();

    let error = Strict::builder("svc")
        .env("STRICTDOTENV_")
        .env_file("tests/scratch/strict.env")
        .strict_env()
        .load()
        .expect_err("the file carries the same ambiguity");
    let message = error.to_string();

    assert!(message.contains("STRICTDOTENV_SVC_MODE"), "{message}");
    assert!(message.contains("strict.env"), "{message}");
}

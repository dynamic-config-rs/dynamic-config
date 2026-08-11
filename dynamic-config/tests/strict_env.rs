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

/// The opted-in invariant holds on the fallback path too: an ambiguous
/// spelling refused during the normal load is refused during recovery.
#[test]
fn recovery_is_held_to_strict_env() {
    #[dynamic_config::dynamic_config]
    #[derive(Debug, Deserialize)]
    struct StrictRecovery {
        #[allow(dead_code)]
        #[serde(default)]
        mode: String,
    }

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/strict-recovery.json",
        r#"{"svc": {"mode": "fast"}}"#,
    )
    .unwrap();

    let builder = StrictRecovery::builder("svc")
        .file("tests/scratch/strict-recovery.json")
        .env("STRICTRECOVER_")
        .strict_env()
        .cache(
            "tests/scratch/strict-recovery-cache.json",
            dynamic_config::CacheMode::Full,
        );
    builder
        .init()
        .expect("the source reads cleanly; the cache is written");

    // The source breaks, and the environment now carries the exact
    // ambiguity strict mode exists for.
    std::fs::write("tests/scratch/strict-recovery.json", "{ not json").unwrap();
    std::env::set_var("STRICTRECOVER_SVC_MODE", "off");

    let error = builder
        .init()
        .expect_err("recovery must not suspend strict_env");
    let message = error.to_string();

    std::env::remove_var("STRICTRECOVER_SVC_MODE");

    assert!(message.contains("STRICTRECOVER_SVC_MODE"), "{message}");
}

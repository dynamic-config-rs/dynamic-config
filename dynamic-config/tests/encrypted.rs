//! Config files that are encrypted on disk.
//!
//! The ciphertext is produced here rather than checked in: a fixture encrypted
//! to a key in the repository would be theatre, and one encrypted to a key that
//! is not would stop working the day it rotated.

#![cfg(all(feature = "age", feature = "json"))]

use std::path::{Path, PathBuf};

use dynamic_config::age::Age;
use dynamic_config::{dynamic_config, ErrorKind};
use serde::Deserialize;

const PASSPHRASE: &str = "correct horse battery staple";

/// Every test in this binary shares one process-wide decryptor, which is the
/// shape the API has on purpose: a key is a process-wide fact.
fn install() {
    use std::sync::Once;

    static ONCE: Once = Once::new();

    ONCE.call_once(|| {
        dynamic_config::set_decryptor(Age::from_passphrase(PASSPHRASE))
            .map_err(|_| "a decryptor was already installed")
            .expect("this is the only installer in this binary");
    });
}

fn encrypt_to(path: &Path, plaintext: &str) {
    let recipient =
        age::scrypt::Recipient::new(age::secrecy::SecretString::from(PASSPHRASE.to_owned()));

    let ciphertext = age::encrypt(&recipient, plaintext.as_bytes()).expect("encrypting works");

    std::fs::create_dir_all(path.parent().expect("a parent")).unwrap();
    std::fs::write(path, ciphertext).unwrap();
}

fn scratch(test: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("dynamic-config-encrypted")
        .join(test);

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();

    directory
}

// The macro takes literals, so each test owns a fixed path under `tests/`.
#[dynamic_config(
    files = ["tests/scratch/enc-config.json", "tests/scratch/enc-secrets.json.age"],
    key = "db",
    env = "DCENC_",
)]
#[derive(Deserialize)]
struct Layered {
    host: String,
    #[config(secret)]
    password: String,
}

#[test]
fn an_encrypted_file_layers_over_a_plain_one() {
    install();

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/enc-config.json",
        r#"{"db": {"host": "localhost", "password": "placeholder"}}"#,
    )
    .unwrap();
    encrypt_to(
        Path::new("tests/scratch/enc-secrets.json.age"),
        r#"{"db": {"password": "hunter2"}}"#,
    );

    Layered::init().expect("both files are readable");

    assert_eq!(Layered::current().host, "localhost", "from the plain file");
    assert_eq!(
        Layered::current().password,
        "hunter2",
        "the encrypted file wins, being listed last"
    );
}

// Its own type and its own files, not `Layered`'s: two tests sharing a
// fixture path race in parallel — one reads the ciphertext mid-rewrite and
// fails with "failed to fill whole buffer". One type, one fixture, per test.
#[dynamic_config(
    files = ["tests/scratch/enc-traced.json", "tests/scratch/enc-traced-secrets.json.age"],
    key = "db",
    env = "DCENCTR_",
)]
#[derive(Deserialize)]
struct Traced {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    #[config(secret)]
    password: String,
}

#[test]
fn a_value_from_an_encrypted_file_is_traced_to_that_file() {
    install();

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write(
        "tests/scratch/enc-traced.json",
        r#"{"db": {"host": "localhost", "password": "placeholder"}}"#,
    )
    .unwrap();
    encrypt_to(
        Path::new("tests/scratch/enc-traced-secrets.json.age"),
        r#"{"db": {"password": "hunter2"}}"#,
    );

    let origin = Traced::source_of("password")
        .expect("resolving works")
        .expect("something supplies it");

    assert!(
        format!("{origin}").contains("enc-traced-secrets.json.age"),
        "not `an inline source`: {origin}"
    );
}

#[dynamic_config(files = ["tests/scratch/enc-optional.json.age"], key = "db", env = "DCENCOPT_")]
#[derive(Debug, Deserialize)]
struct Optional {
    #[allow(dead_code)]
    host: String,
}

#[test]
fn an_encrypted_file_that_is_not_there_is_skipped_like_any_other() {
    install();

    let _ = std::fs::remove_file("tests/scratch/enc-optional.json.age");

    // Nothing supplies `host`, so this fails as *missing* rather than as a
    // decryption failure — the file being absent is not an error, it is an
    // absent file.
    let error = Optional::load().expect_err("nothing supplies host");

    assert_eq!(error.kind(), ErrorKind::Missing, "{error}");

    std::env::set_var("DCENCOPT_DB_HOST", "from-the-environment");
    Optional::init().expect("and the environment can fill the gap");
    std::env::remove_var("DCENCOPT_DB_HOST");
}

#[dynamic_config(files = ["tests/scratch/enc-wrong.json.age"], key = "db")]
#[derive(Debug, Deserialize)]
struct WrongKey {
    #[allow(dead_code)]
    host: String,
}

#[test]
fn a_file_this_key_cannot_open_says_so_and_names_the_file() {
    install();

    // Encrypted to a different passphrase than the installed decryptor holds.
    let recipient = age::scrypt::Recipient::new(age::secrecy::SecretString::from(
        "a different one".to_owned(),
    ));
    let ciphertext = age::encrypt(&recipient, br#"{"db": {"host": "x"}}"#).unwrap();

    std::fs::create_dir_all("tests/scratch").unwrap();
    std::fs::write("tests/scratch/enc-wrong.json.age", ciphertext).unwrap();

    let error = WrongKey::load().expect_err("the key does not match");

    assert_eq!(error.kind(), ErrorKind::Decrypt);
    assert!(error.to_string().contains("enc-wrong.json.age"), "{error}");
    assert!(error.to_string().contains("recipient"), "{error}");
}

#[dynamic_config(
    files = ["tests/scratch/enc-profile.json.age"],
    key = "db",
    profile_env = "DCENC_PROFILE",
)]
#[derive(Deserialize)]
struct Profiled {
    host: String,
}

#[test]
fn a_profile_variant_goes_under_the_suffix_not_after_it() {
    install();

    encrypt_to(
        Path::new("tests/scratch/enc-profile.json.age"),
        r#"{"db": {"host": "base"}}"#,
    );
    // `secrets.json.age` + `production` is `secrets.production.json.age`.
    encrypt_to(
        Path::new("tests/scratch/enc-profile.production.json.age"),
        r#"{"db": {"host": "production"}}"#,
    );

    Profiled::init().expect("the base file alone works");
    assert_eq!(Profiled::current().host, "base");

    std::env::set_var("DCENC_PROFILE", "production");
    Profiled::init().expect("and the variant layers over it");
    assert_eq!(Profiled::current().host, "production");
    std::env::remove_var("DCENC_PROFILE");
}

#[test]
fn armored_and_binary_files_both_read() {
    install();

    let directory = scratch("armor");
    let recipient =
        age::scrypt::Recipient::new(age::secrecy::SecretString::from(PASSPHRASE.to_owned()));

    let armored = age::encrypt_and_armor(&recipient, br#"{"db": {"host": "armored"}}"#).unwrap();
    let path = directory.join("armored.json.age");
    std::fs::write(&path, &armored).unwrap();

    let sources = [dynamic_config::Source::encrypted(
        path.to_str().unwrap(),
        dynamic_config::Format::Json,
    )];

    #[derive(Deserialize)]
    struct Db {
        host: String,
    }

    let db: Db = dynamic_config::load(&dynamic_config::LoadSpec::new("db", &sources))
        .expect("armor is detected rather than configured");

    assert_eq!(db.host, "armored");
}

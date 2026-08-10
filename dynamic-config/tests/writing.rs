//! Writing configuration back out: refusing to overwrite, and encrypting.

#![cfg(feature = "json")]

use std::path::PathBuf;

use dynamic_config::{dynamic_config, ErrorKind};
use serde::{Deserialize, Serialize};

#[dynamic_config(files = ["tests/fixtures/base.json"], key = "db", save)]
#[derive(Deserialize, Serialize)]
struct Written {
    host: String,
    port: u16,
}

fn scratch(test: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("dynamic-config-writing")
        .join(test);

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();

    directory
}

#[test]
fn save_new_writes_when_there_is_nothing_there() {
    let path = scratch("fresh").join("config.json");

    Written::load()
        .unwrap()
        .save_new(&path)
        .expect("nothing is there");

    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("localhost"));
}

#[test]
fn save_new_refuses_rather_than_replacing() {
    let path = scratch("occupied").join("config.json");

    std::fs::write(&path, "written by hand").unwrap();

    let error = Written::load()
        .unwrap()
        .save_new(&path)
        .expect_err("the file is already there");

    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "written by hand",
        "a setup wizard must not replace what somebody wrote"
    );
}

#[test]
fn save_still_replaces_because_that_is_what_it_is_for() {
    let path = scratch("replace").join("config.json");

    std::fs::write(&path, "stale").unwrap();

    Written::load()
        .unwrap()
        .save(&path)
        .expect("save overwrites");

    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("localhost"));
}

#[cfg(unix)]
#[test]
fn a_file_from_save_new_is_created_private() {
    use std::os::unix::fs::PermissionsExt;

    let path = scratch("mode").join("config.json");

    Written::load().unwrap().save_new(&path).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;

    assert_eq!(mode, 0o600, "mode was {mode:o}");
}

#[cfg(feature = "age")]
mod encrypted {
    use dynamic_config::age::{Age, Recipients};

    use super::*;

    /// What it wrote must come back through the reading side — an encrypted
    /// file only one of the two halves understands is worse than none.
    #[test]
    fn what_it_writes_is_what_the_reading_side_reads() {
        let path = scratch("round-trip").join("secrets.json.age");
        let identity = age::x25519::Identity::generate();
        let public = identity.to_public().to_string();

        let recipients = Recipients::from_public_keys([public]).expect("a real recipient");

        Written::load()
            .unwrap()
            .save_encrypted(&path, &recipients)
            .expect("encrypting works");

        let ciphertext = std::fs::read(&path).unwrap();

        assert!(
            !String::from_utf8_lossy(&ciphertext).contains("localhost"),
            "the plaintext must not be on disk"
        );

        let key = age::secrecy::ExposeSecret::expose_secret(&identity.to_string()).to_owned();
        let reader = Age::from_key(&key).expect("the identity parses");

        let plaintext = dynamic_config::Decryptor::decrypt(&reader, &ciphertext)
            .expect("the recipient can open it");

        assert!(String::from_utf8_lossy(&plaintext).contains("localhost"));
    }

    #[test]
    fn a_passphrase_round_trips_too() {
        let path = scratch("passphrase").join("secrets.json.age");

        Written::load()
            .unwrap()
            .save_encrypted(&path, &Recipients::from_passphrase("hunter2"))
            .expect("encrypting works");

        let ciphertext = std::fs::read(&path).unwrap();
        let plaintext =
            dynamic_config::Decryptor::decrypt(&Age::from_passphrase("hunter2"), &ciphertext)
                .expect("the passphrase opens it");

        assert!(String::from_utf8_lossy(&plaintext).contains("localhost"));
    }

    #[test]
    fn encrypting_to_nobody_is_refused() {
        let error = Recipients::from_public_keys(Vec::<String>::new())
            .expect_err("a file nobody can read is not a file worth writing");

        assert!(error.to_string().contains("nobody"), "{error}");
    }

    #[test]
    fn something_that_is_not_a_recipient_names_itself() {
        let error = Recipients::from_public_keys(["not-an-age-key"]).expect_err("it is not one");

        assert!(error.to_string().contains("not-an-age-key"), "{error}");
    }
}

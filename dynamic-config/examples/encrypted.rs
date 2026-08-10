//! A `secrets.json.age` that lives in the repository, decrypted at load time.
//!
//! ```text
//! cargo run -p dynamic-config --example encrypted --features age,json
//! ```
//!
//! The example generates a key and encrypts a file with it, so it runs with no
//! setup. A real deployment does that part once, with the `age` CLI:
//!
//! ```sh
//! age-keygen -o key.txt
//! age -R recipients.txt -o secrets.json.age secrets.json && rm secrets.json
//! ```
//!
//! and then sets `SOPS_AGE_KEY_FILE=/etc/myapp/key.txt` on the machine.

use std::path::PathBuf;

use dynamic_config::age::Age;
use dynamic_config::dynamic_config;
use serde::Deserialize;

const DIRECTORY: &str = "/tmp/dynamic-config-encrypted-example";

#[dynamic_config(
    files = [
        "/tmp/dynamic-config-encrypted-example/config.json",
        "/tmp/dynamic-config-encrypted-example/secrets.json.age",
    ],
    key = "db",
    env = "APP_",
)]
#[derive(Deserialize)]
struct DbConfig {
    host: String,
    port: u16,
    #[config(secret)]
    password: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(DIRECTORY);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;

    // ---------------------------------------------------------------------
    // Setup a deployment does once, with the CLI.
    // ---------------------------------------------------------------------
    let identity = age::x25519::Identity::generate();

    let key_file = directory.join("key.txt");
    std::fs::write(
        &key_file,
        format!(
            "# created by the example\n{}\n",
            age::secrecy::ExposeSecret::expose_secret(&identity.to_string())
        ),
    )?;

    std::fs::write(
        directory.join("config.json"),
        r#"{"db": {"host": "db.internal", "port": 5432, "password": "unset"}}"#,
    )?;

    let ciphertext = age::encrypt(&identity.to_public(), br#"{"db": {"password": "hunter2"}}"#)?;
    std::fs::write(directory.join("secrets.json.age"), &ciphertext)?;

    println!("on disk:");
    println!("  config.json         plain, safe to read");
    println!(
        "  secrets.json.age    {} bytes of ciphertext",
        ciphertext.len()
    );
    println!(
        "  (grep finds nothing: {})\n",
        !String::from_utf8_lossy(&ciphertext).contains("hunter2")
    );

    // ---------------------------------------------------------------------
    // What the program does: install a key, then load as usual.
    // ---------------------------------------------------------------------
    std::env::set_var("SOPS_AGE_KEY_FILE", &key_file);

    // `from_environment` reads SOPS_AGE_KEY_FILE, AGE_IDENTITY_FILE or
    // AGE_SECRET_KEY, in that order. Installed once per process, because a
    // decryption key is a process-wide fact.
    dynamic_config::set_decryptor(Age::from_environment()?)
        .map_err(|_| "a decryptor was already installed")?;

    DbConfig::init()?;

    let config = DbConfig::current();

    println!("loaded:");
    println!("  host     = {}", config.host);
    println!("  port     = {}", config.port);
    println!("  password = {} (from the encrypted file)", {
        assert_eq!(config.password, "hunter2");
        "***"
    });

    // The encrypted file is a file like any other, and the trace says so.
    println!(
        "\n  password came {}",
        DbConfig::source_of("password")?.expect("something supplies it")
    );
    println!(
        "  host came {}",
        DbConfig::source_of("host")?.expect("something supplies it")
    );

    // And the environment still wins over both.
    std::env::set_var("APP_DB_PASSWORD", "from-the-machine");
    DbConfig::init()?;
    println!(
        "\nwith APP_DB_PASSWORD set, password came {}",
        DbConfig::source_of("password")?.expect("something supplies it")
    );
    std::env::remove_var("APP_DB_PASSWORD");

    // ---------------------------------------------------------------------
    // The failure worth seeing: the wrong key.
    // ---------------------------------------------------------------------
    let stranger = age::x25519::Identity::generate();
    let unreadable = age::encrypt(&stranger.to_public(), br#"{"db": {}}"#)?;
    std::fs::write(directory.join("secrets.json.age"), unreadable)?;

    match DbConfig::load() {
        Ok(_) => println!("\nunexpectedly readable"),
        Err(error) => println!("\nwith a file this key cannot open:\n  {error}"),
    }

    let _ = std::fs::remove_dir_all(&directory);

    Ok(())
}

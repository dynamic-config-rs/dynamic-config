//! The encrypted last-known-good cache: full fidelity, nothing readable
//! at rest.
//!
//! Its own binary rather than a module in `cache.rs`'s suite, because the
//! installed decryptor is process-global (`set_decryptor` is first-wins)
//! and these tests need a toy one all to themselves.

#![cfg(all(feature = "json", feature = "decrypt"))]

use std::fs;

use dynamic_config::{Builder, Error};
use serde::Deserialize;

/// A reversible toy: XOR with a fixed byte, behind a marker prefix.
///
/// Not cryptography — the property under test is the *plumbing*: what the
/// cache writes goes through the encryptor, what recovery reads goes
/// through the decryptor, and plaintext never touches the disk. A real
/// deployment installs `age`; the toy keeps the suite free of key files.
struct Toy;

const MARKER: &[u8] = b"toy-encrypted:";

impl dynamic_config::Encryptor for Toy {
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = MARKER.to_vec();
        out.extend(plaintext.iter().map(|byte| byte ^ 0x5a));
        Ok(out)
    }

    fn describe(&self) -> String {
        "the toy encryptor".to_owned()
    }
}

struct ToyDecryptor;

impl dynamic_config::Decryptor for ToyDecryptor {
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
        let body = ciphertext
            .strip_prefix(MARKER)
            .ok_or_else(|| Error::invalid("not the toy's ciphertext"))?;

        Ok(body.iter().map(|byte| byte ^ 0x5a).collect())
    }

    fn describe(&self) -> String {
        "the toy decryptor".to_owned()
    }
}

#[derive(Debug, Deserialize)]
struct Db {
    host: String,
    password: String,
}

#[test]
fn the_cache_encrypts_at_rest_and_recovers_in_full() {
    let _ = dynamic_config::set_decryptor(ToyDecryptor);

    fs::create_dir_all("tests/scratch").unwrap();
    let source = "tests/scratch/enc-cache-source.json";
    let cache = "tests/scratch/enc-cache-last.json.age";
    let _ = fs::remove_file(cache);

    fs::write(
        source,
        r#"{"db": {"host": "db.internal", "password": "planted-s3cret"}}"#,
    )
    .unwrap();

    // A cell to install into: the cache is written on `init`, and a plain
    // `Dynamic` is the least ceremony that has one.
    let dynamic = dynamic_config::Dynamic::new(
        Builder::<Db>::new("db")
            .file(source)
            .cache_encrypted(cache, Toy),
    );
    dynamic.init().expect("the initial load succeeds");

    // At rest: the marker is there, the secret is not — full fidelity on
    // the inside, nothing readable on the outside.
    let bytes = fs::read(cache).expect("the cache was written");
    assert!(bytes.starts_with(MARKER), "written through the encryptor");
    assert!(
        !bytes
            .windows(b"planted-s3cret".len())
            .any(|window| window == b"planted-s3cret"),
        "the plaintext secret must not appear in the cache file"
    );

    // Recovery: the sources are gone, and — unlike a redacted cache — the
    // environment supplies nothing. Everything comes back from the cache.
    fs::remove_file(source).unwrap();
    fs::write(source, "{not json").unwrap();

    let recovered = dynamic_config::Dynamic::new(
        Builder::<Db>::new("db")
            .file(source)
            .cache_encrypted(cache, Toy),
    );
    recovered
        .init()
        .expect("recovery from the encrypted cache succeeds");

    let db = recovered.current().expect("recovered and installed");
    assert_eq!(db.host, "db.internal");
    assert_eq!(db.password, "planted-s3cret", "full fidelity is the point");
}

#[test]
fn an_encrypted_cache_path_must_carry_the_format_under_the_suffix() {
    let _ = dynamic_config::set_decryptor(ToyDecryptor);

    fs::create_dir_all("tests/scratch").unwrap();
    let source = "tests/scratch/enc-cache-badpath.json";
    fs::write(source, r#"{"db": {"host": "x", "password": "y"}}"#).unwrap();

    // `last.age` names no inner format; the refusal happens at write time
    // and is reported as the cache warning path — `init` itself succeeds,
    // and nothing lands on disk.
    let dynamic = dynamic_config::Dynamic::new(
        Builder::<Db>::new("db")
            .file(source)
            .cache_encrypted("tests/scratch/enc-cache-bad", Toy),
    );
    dynamic
        .init()
        .expect("a cache that cannot write is a warning");
    assert!(!std::path::Path::new("tests/scratch/enc-cache-bad").exists());
}

/// The last cache call wins, in both directions — an encryptor left armed
/// after a plaintext `.cache(..)` would write a full encrypted document
/// where redaction was asked for.
#[test]
fn the_last_cache_call_wins_in_both_directions() {
    let _ = dynamic_config::set_decryptor(ToyDecryptor);

    fs::create_dir_all("tests/scratch").unwrap();
    let source = "tests/scratch/enc-cache-order.json";
    fs::write(
        source,
        r#"{"db": {"host": "h", "password": "order-s3cret"}}"#,
    )
    .unwrap();

    // Encrypted, then plaintext-full: the plaintext file is what exists.
    let plain = "tests/scratch/enc-cache-order-plain.json";
    let _ = fs::remove_file(plain);
    let dynamic = dynamic_config::Dynamic::new(
        Builder::<Db>::new("db")
            .file(source)
            .cache_encrypted("tests/scratch/enc-cache-order.json.age", Toy)
            .cache(plain, dynamic_config::CacheMode::Full),
    );
    dynamic.init().expect("init succeeds");
    let bytes = fs::read(plain).expect("the plaintext cache was written");
    assert!(
        !bytes.starts_with(MARKER),
        "the later `.cache(..)` must not write through the earlier encryptor"
    );

    // Plaintext, then encrypted: the encrypted file is what exists.
    let encrypted = "tests/scratch/enc-cache-order-enc.json.age";
    let _ = fs::remove_file(encrypted);
    let dynamic = dynamic_config::Dynamic::new(
        Builder::<Db>::new("db")
            .file(source)
            .cache(plain, dynamic_config::CacheMode::Full)
            .cache_encrypted(encrypted, Toy),
    );
    dynamic.init().expect("init succeeds");
    assert!(fs::read(encrypted)
        .expect("the encrypted cache was written")
        .starts_with(MARKER));
}
